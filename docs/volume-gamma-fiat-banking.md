# VOLUME GAMMA — `mm-fiat` & Banking/Utility Engine

**Version:** 2.4.0 · **Owner:** Payments Engineering · **Scope:** Flutterwave virtual accounts
(NUBANs), funding webhooks, Yellow Card FX quoting, NGN payouts.

---

## 1. Architectural Overview & Technical Scope

`mm-fiat` wraps two external rails behind one internal API:

1. **Flutterwave** — dedicated virtual NUBAN issuance (Wema/Sterling/Bank9ja), inbound transfer
   webhooks, outbound NGN transfers (payouts).
2. **Yellow Card** — live USDT/NGN mid-market rate, polled every 60 s, cached in Redis, used by
   every quote in the system after applying the operational spread.

Hard rules:
- No wallet mutation happens inside this crate. It emits *intents*; `mm-api` applies them via
  `mm-db` transactions (Vol 2 §4). This keeps Law 3 (SQLx = all ledger mutations).
- Every webhook is verified (`verif-hash` HMAC) and idempotent
  (`tx_ref` uniqueness gate) before any credit.

## 2. Mathematical Formulation — Quote & Spread

```
mid            = yellow_card_mid(USDT_NGN)                  # refreshed every 60 s
user_buy_quote = mid * (1 - spread)                          # user sells USDT to us
user_sell_quote= mid * (1 + spread)                          # user buys USDT from us
spread         ∈ [1.5%, 3.0%]   (ops-configurable per tier; TIER_1: 3.0%, TIER_2: 1.5%)
stale_after    = 120 s -> quotes rejected until refresh succeeds
gift_card_rate = catalogue_rate_per_dollar (brand/country/format specific, NOT fx-derived)
```

Rounding: NGN amounts round half-up to 2 dp at the FINAL step only — intermediate math keeps
full precision (`rust_decimal`).

## 3. Complete Implementation

### 3.1 `crates/mm-fiat/src/lib.rs`

```rust
pub mod flutterwave;
pub mod quotes;
pub mod types;

pub use flutterwave::{FlwClient, FlwError};
pub use quotes::QuoteEngine;
```

### 3.2 `src/types.rs`

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum KycTier { Unverified, Tier1, Tier2 }

#[derive(Debug, Clone, Serialize)]
pub struct VirtualAccount {
    pub bank_name: String,
    pub account_number: String,
    pub account_name: String,
    pub flw_order_ref: String,
}

#[derive(Debug, Clone)]
pub struct FxQuote {
    pub pair: &'static str,
    pub mid: rust_decimal::Decimal,
    pub buy: rust_decimal::Decimal,
    pub sell: rust_decimal::Decimal,
    pub fetched_at_unix: i64,
}
```

### 3.3 `src/flutterwave.rs`

```rust
use crate::types::VirtualAccount;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::json;
use sha2::Sha256;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlwError {
    #[error("transport failure: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("flutterwave rejected request [{status}]: {body}")]
    Rejected { status: u16, body: String },
    #[error("webhook signature mismatch")]
    BadSignature,
}

type HmacSha256 = Hmac<Sha256>;

pub struct FlwClient {
    http: Client,
    secret_key: String,
    webhook_hash: String,
    base: String,
}

pub struct FlwWebhook {
    pub tx_ref: String,
    pub flw_id: String,
    pub amount: f64,
    pub currency: String,
    pub payer_bank: Option<String>,
    pub payer_account: Option<String>,
}

impl FlwClient {
    pub fn new(secret_key: String, webhook_hash: String) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("static client"),
            secret_key,
            webhook_hash,
            base: "https://api.flutterwave.com/v3".into(),
        }
    }

    /// Issues a dedicated NUBAN for a user (called on Tier-1 KYC completion).
    pub async fn create_virtual_account(
        &self,
        user_id: uuid::Uuid,
        full_name: &str,
        bvn_hash: &str,
    ) -> Result<VirtualAccount, FlwError> {
        let body = json!({
            "email": format!("{}@users.middleman.africa", user_id),
            "is_permanent": true,
            "bvn": bvn_hash,
            "tx_ref": format!("MM-VA-{}", user_id),
            "firstname": full_name.split(' ').next().unwrap_or("MiddleMan"),
            "lastname": full_name.split(' ').nth(1).unwrap_or("User"),
        });

        let resp = self
            .http
            .post(format!("{}/virtual-account-numbers", self.base))
            .bearer_auth(&self.secret_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(FlwError::Rejected { status: status.as_u16(), body: text });
        }

        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| FlwError::Rejected { status: 500, body: text })?;
        let d = &v["data"];
        Ok(VirtualAccount {
            bank_name: d["bank_name"].as_str().unwrap_or_default().to_string(),
            account_number: d["account_number"].as_str().unwrap_or_default().to_string(),
            account_name: d["account_name"].as_str().unwrap_or_default().to_string(),
            flw_order_ref: d["order_ref"].as_str().unwrap_or_default().to_string(),
        })
    }

    /// Constant-time verification of the `verif-hash` header.
    pub fn verify_webhook_signature(&self, raw_body: &[u8], provided: &str) -> Result<(), FlwError> {
        let mut mac = HmacSha256::new_from_slice(self.webhook_hash.as_bytes())
            .map_err(|_| FlwError::BadSignature)?;
        mac.update(raw_body);
        let expected = hex::encode(mac.finalize().into_bytes());
        let a = expected.into_bytes();
        let b = provided.trim().as_bytes().to_vec();
        if a.len() == b.len() && constant_time_eq(&a, &b) { Ok(()) } else { Err(FlwError::BadSignature) }
    }

    pub fn parse_webhook(raw_body: &[u8]) -> Result<FlwWebhook, serde_json::Error> {
        let v: serde_json::Value = serde_json::from_slice(raw_body)?;
        let data = &v["data"];
        Ok(FlwWebhook {
            tx_ref: data["tx_ref"].as_str().unwrap_or_default().to_string(),
            flw_id: data["id"].as_u64().unwrap_or(0).to_string(),
            amount: data["amount"].as_f64().unwrap_or(0.0),
            currency: data["currency"].as_str().unwrap_or("NGN").to_string(),
            payer_bank: data["payment_type"].as_str().map(str::to_string),
            payer_account: None,
        })
    }

    /// Outbound NGN payout to a user's bank account.
    pub async fn send_payout(
        &self,
        reference: &str,
        bank_code: &str,
        account_number: &str,
        amount_ngn: f64,
        narration: &str,
    ) -> Result<String, FlwError> {
        let body = json!({
            "account_bank": bank_code,
            "account_number": account_number,
            "amount": amount_ngn,
            "narration": narration,
            "currency": "NGN",
            "reference": reference,
            "callback_url": "https://middleman.africa/api/v1/fiat/payout-callback"
        });
        let resp = self.http.post(format!("{}/transfers", self.base))
            .bearer_auth(&self.secret_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(FlwError::Rejected { status: status.as_u16(), body: text });
        }
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| FlwError::Rejected { status: 500, body: text })?;
        Ok(v["data"]["id"].as_i64().unwrap_or(0).to_string())
    }

    /// Bank list cache for the admin desk (fetched daily).
    pub async fn fetch_banks(&self) -> Result<Vec<(String, String)>, FlwError> {
        let resp = self.http.get(format!("{}/banks/NG", self.base))
            .bearer_auth(&self.secret_key).send().await?;
        let v: serde_json::Value = resp.json().await?;
        Ok(v["data"].as_array().cloned().unwrap_or_default().iter()
            .filter_map(|b| Some((
                b["code"].as_str()?.to_string(),
                b["name"].as_str()?.to_string(),
            )))
            .collect())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) { diff |= x ^ y; }
    diff == 0
}
```

### 3.4 `src/quotes.rs`

```rust
use crate::types::FxQuote;
use redis::AsyncCommands;
use rust_decimal::Decimal;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QuoteError {
    #[error("rate stale (>120s); refusing to quote")]
    StaleRate,
    #[error("redis failure: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("upstream rate unavailable: {0}")]
    Upstream(String),
}

const KEY_LAST_RATE: &str = "fx:last:USDT_NGN";       // JSON FxQuote
const STALE_SECS: i64 = 120;

pub struct QuoteEngine {
    redis: redis::Client,
    http: reqwest::Client,
    yellow_card_base: String,
}

impl QuoteEngine {
    pub fn new(redis_url: &str) -> Self {
        Self {
            redis: redis::Client::open(redis_url).expect("valid redis url"),
            http: reqwest::Client::new(),
            yellow_card_base: std::env::var("YELLOW_CARD_BASE")
                .unwrap_or_else(|_| "https://sandbox.api.yellowcard.io".into()),
        }
    }

    /// Background task: poll every 60 s, publish to Redis.
    pub async fn run_refresh_loop(self) -> ! {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = self.refresh_once().await {
                eprintln!("fx refresh failed: {e}");
            }
        }
    }

    async fn refresh_once(&self) -> Result<(), QuoteError> {
        let resp = self.http.get(format!("{}/v1/rates?pair=USDTNGN", self.yellow_card_base))
            .header("X-YC-API-Key", std::env::var("YELLOW_CARD_API_KEY").unwrap_or_default())
            .send().await.map_err(|e| QuoteError::Upstream(e.to_string()))?;

        let v: serde_json::Value = resp.json().await.map_err(|e| QuoteError::Upstream(e.to_string()))?;
        let mid = v["data"]["rate"].as_str()
            .and_then(|s| s.parse::<Decimal>().ok())
            .ok_or_else(|| QuoteError::Upstream("unparseable rate".into()))?;

        let now = chrono::Utc::now().timestamp();
        let quote = FxQuote { pair: "USDT_NGN", mid, buy: mid, sell: mid, fetched_at_unix: now };
        let conn = self.redis.get_multiplexed_async_connection().await?;
        let _: () = conn.set_ex(KEY_LAST_RATE, serde_json::to_string(&quote).unwrap(), 3600).await?;
        Ok(())
    }

    /// Applies tier spread and returns both directions.
    pub async fn quote_with_spread(
        &self,
        tier_spread_bps: i64,          // 150..300 basis points
    ) -> Result<(Decimal, Decimal), QuoteError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let raw: Option<String> = conn.get(KEY_LAST_RATE).await?;
        let q: FxQuote = raw
            .and_then(|r| serde_json::from_str(&r).ok())
            .ok_or(QuoteError::StaleRate)?;

        let now = chrono::Utc::now().timestamp();
        if now - q.fetched_at_unix > STALE_SECS { return Err(QuoteError::StaleRate); }

        let sp = Decimal::new(tier_spread_bps, 4); // bps -> fraction
        let buy = (q.mid * (Decimal::ONE - sp)).round_dp(2);
        let sell = (q.mid * (Decimal::ONE + sp)).round_dp(2);
        Ok((buy, sell))
    }
}
```

## 4. Data Schemas & Structural Interfaces

### Webhook route contract (`mm-api`)

```
POST /api/v1/fiat/flw-webhook
Headers: verif-hash: <FLW_WEBHOOK_HASH>
Body:    Flutterwave charge.completed payload (JSON)

Processing order:
1. verify_webhook_signature(raw_bytes)      -> 401 on mismatch
2. parse_webhook
3. INSERT processed_flw_events(tx_ref PK)   -> duplicate => 200 OK no-op
4. resolve tx_ref -> user (format MM-TOPUP-{user_id} or VA order lookup)
5. mm-db atomic credit + transactions row FIAT_TOPUP SUCCESS
6. notify user via Vol 1 Outbound ("₦X credited")
```

Additional table (migration 0002):

```sql
CREATE TABLE processed_flw_events (
    tx_ref VARCHAR(128) PRIMARY KEY,
    flw_id VARCHAR(32),
    credited_amount_ngn NUMERIC(18,2) NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

| Internal call | Signature |
|---|---|
| Issue NUBAN | `create_virtual_account(user_id, full_name, bvn_hash) -> VirtualAccount` |
| Verify hook | `verify_webhook_signature(raw, verif_hash_header) -> Result` |
| Quote | `quote_with_spread(tier_spread_bps) -> (buy_rate, sell_rate)` |
| Payout | `send_payout(reference, bank_code, acct_no, amount, narration) -> flw_transfer_id` |

## 5. Error Handling, Retry & Reconciliation Policies

| Condition | Policy |
|---|---|
| Webhook signature mismatch | 401 immediately; log IP; alert ops (possible probing) |
| Duplicate webhook | Idempotent 200 (processed_flw_events PK) |
| Credit fails AFTER signature ok | Retryable: return 500 so Flutterwave redelivers (their retry schedule) |
| Rate stale > 120 s | All quotes fail-closed; FSM replies "rates updating, try shortly" |
| Payout `reference` reused | Flutterwave rejects duplicates; we map to `PAYOUT_DUPLICATE` |
| Withdrawal ≥ ₦500,000 | Held in PROCESSING pending admin approval (Vol Eta guard) |

Daily reconciliation job compares `SUM(FIAT_TOPUP credits) - SUM(payout debits)` against
Flutterwave settlements report CSV; drift > ₦100 pages ops.

## 6. Verification Test Cases & Command Sequences

```bash
# VG-T1: signature verification vectors
cargo test -p mm-fiat webhook_signature
# includes tampered-body and wrong-header cases asserting BadSignature

# VG-T2: idempotent webhook replay
curl -s -X POST localhost:3000/api/v1/fiat/flw-webhook \
  -H "verif-hash: $FLW_WEBHOOK_HASH" -H 'Content-Type: application/json' \
  -d @fixtures/flw_credit.json           # expect 200 credited
curl -s -X POST localhost:3000/api/v1/fiat/flw-webhook \
  -H "verif-hash: $FLW_WEBHOOK_HASH" -d @fixtures/flw_credit.json
# second call: 200, but balance unchanged (single credit in ledger)

# VG-T3: quote freshness gate
redis-cli DEL fx:last:USDT_NGN
cargo test -p mm-fiat stale_rate_rejects

# VG-T4: NUBAN issuance end-to-end (sandbox)
FLUTTERWAVE_SECRET_KEY=$FLW_SANDBOX cargo test -p mm-fiat --ignored issue_nuban_sandbox

# VG-T5: reconciliation zero-drift
psql $DATABASE_URL -f crates/mm-db/sql/reconcile.sql   # returns 0 rows
```
