# VOLUME DELTA — `mm-crypto` & Degen Trading Engine

**Version:** 2.4.0 · **Owner:** Web3 Engineering · **Scope:** Deterministic wallet provisioning,
Jupiter v6 swaps, GoPlus security radar, internal P2P ledger transfers.

---

## 1. Architectural Overview & Technical Scope

`mm-crypto` exposes four capabilities to the FSM:

1. **Provisioning** — generate a fresh keypair per chain at onboarding; encrypt via `mm-vault`
   before the plaintext ever leaves the signing module's scope.
2. **Swaps** — Solana via Jupiter v6 (`quote` → `swap` → sign → send → confirm). EVM swaps
   (LI.FI / Uniswap) are Phase 2; the trait surface below already anticipates them.
3. **Security radar** — GoPlus pre-trade gate; honeypot or `sell_tax > 10%` blocks execution.
4. **P2P transfers** — instant internal ledger moves keyed by phone number with on-chain
   settlement fallback.

Chains at MVP: **Solana** (swaps + custody), EVM read-only radar. Tron/TON reserved.

## 2. Mathematical & Security Formulations

### 2.1 Slippage & Minimum Output

```
minOut = quoteOut * (1 - slippage_bps / 10000)        slippage_bps ∈ [50 .. 300] default 150
```

Execution aborts if route refresh deviates > 20% from the displayed quote (sandwich defense).

### 2.2 Confirmation Semantics (Solana)

```
confirmed := transaction status reaches 'confirmed' (1 confirmation supermajority)
deadline  := 60 s from broadcast; else status=FAILED_TIMEOUT and funds un-reserved back to user
fee model : priority micro-lamports capped at 0.00005 SOL per tx; platform fee 0.5% of input,
            taken as a split route leg to treasury wallet.
```

### 2.3 Radar Gate (GoPlus)

```
BLOCK trade iff: is_honeypot == true
              OR sell_tax > 10%
              OR buy_tax  > 15%
              OR is_mintable == "1" AND liquidity_usd < 25_000
WARN user (require explicit confirm) iff: owner_can_change_balance OR lp_holders < 5
```

## 3. Complete Implementation

### 3.1 `crates/mm-crypto/src/lib.rs`

```rust
pub mod goplus;
pub mod jupiter;
pub mod keys;
pub mod p2p;
pub mod radar;

pub use keys::{ChainType, KeyProvisioner};
```

Key crates: `solana-sdk = "1.18"`, `spl-associated-token-account`, `rust_decimal`, `alloy-primitives`
(read-only helpers), `reqwest`, `serde_json`.

### 3.2 `src/keys.rs` — provisioning

```rust
use mm_vault::VaultAead;
use rand::RngCore;
use solana_sdk::signature::{Keypair, Signer};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProvisionError {
    #[error("unsupported chain")]
    UnsupportedChain,
    #[error("vault failure: {0}")]
    Vault(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "varchar", rename_all = "UPPERCASE")]
pub enum ChainType { Evm, Solana, Tron, Ton }

impl ChainType {
    pub fn as_str(&self) -> &'static str {
        match self { ChainType::Evm => "EVM", ChainType::Solana => "SOLANA",
                     ChainType::Tron => "TRON", ChainType::Ton => "TON" }
    }
}

pub struct KeyProvisioner<'a> {
    vault: &'a VaultAead,
}

impl<'a> KeyProvisioner<'a> {
    pub fn new(vault: &'a VaultAead) -> Self { Self { vault } }

    /// Generates a keypair for the chain, encrypts it bound to (user_id, chain),
    /// returns (public_address, encrypted_envelope). Plaintext exists only inside this scope.
    pub fn provision_solana(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<(String, String), ProvisionError> {
        let mut seed = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        let kp = Keypair::from_base58_string(&bs58::encode(seed).into_string());
        let pubkey = kp.pubkey().to_string();
        let secret_bytes = kp.to_bytes();

        let aad = format!("mm:vault:v1:{}:SOLANA", user_id);
        let envelope = self.vault.encrypt(&secret_bytes, &aad)
            .map_err(|e| ProvisionError::Vault(e.to_string()))?;

        Ok((pubkey, envelope))
    }

    /// Decrypts into a zeroizing buffer strictly inside caller's signing scope.
    pub fn load_solana_signer(
        &self,
        user_id: uuid::Uuid,
        envelope: &str,
    ) -> Result<mm_vault::SensitiveKeyBuffer, ProvisionError> {
        let aad = format!("mm:vault:v1:{}:SOLANA", user_id);
        self.vault.decrypt_sensitive(envelope, &aad)
            .map_err(|e| ProvisionError::Vault(e.to_string()))
    }
}
```

### 3.3 `src/jupiter.rs` — swap engine

```rust
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SwapError {
    #[error("quote unavailable: {0}")]
    Quote(String),
    #[error("route deviation exceeds guard ({observed_bps}bps vs locked {locked_bps}bps)")]
    RouteDeviation { observed_bps: u64, locked_bps: u64 },
    #[error("radar blocked: {reason}")]
    RadarBlocked { reason: String },
    #[error("broadcast failed: {0}")]
    Broadcast(String),
    #[error("confirmation timeout after 60s: signature {0}")]
    ConfirmTimeout(String),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JupQuote {
    pub in_amount: String,
    pub out_amount: String,
    pub price_impact_pct: String,
    #[serde(rename = "routePlan")]
    pub route_plan: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct SwapRequest {
    pub user_id: uuid::Uuid,
    pub input_mint: String,      // e.g. USDC mint or USDT mint on Solana
    pub output_mint: String,     // SOL mint
    pub amount_raw: u64,         // base units
    pub slippage_bps: u16,
    pub taker_pubkey: String,
}

pub struct JupiterClient {
    http: reqwest::Client,
    api_base: String,
    rpc: solana_client::nonblocking::rpc_client::RpcClient,
}

const MINT_SOL:  &str = "So11111111111111111111111111111111111111112";
const MINT_USDT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const TREASURY:  &str = "TREASURY_WALLET_PUBKEY_PLACEHOLDER_SET_VIA_ENV";

impl JupiterClient {
    pub fn new(rpc_url: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build().expect("static client"),
            api_base: "https://quote-api.jup.ag/v6".into(),
            rpc: solana_client::nonblocking::rpc_client::RpcClient::new_with_commitment(
                rpc_url, solana_client::rpc_config::RpcCommitmentConfig::Confirmed),
        }
    }

    /// Fetch best route quote.
    pub async fn quote(&self, req: &SwapRequest) -> Result<JupQuote, SwapError> {
        let resp = self.http.get(format!("{}/quote", self.api_base))
            .query(&[
                ("inputMint", req.input_mint.as_str()),
                ("outputMint", req.output_mint.as_str()),
                ("amount", &req.amount_raw.to_string()),
                ("slippageBps", &req.slippage_bps.to_string()),
                ("onlyDirectRoutes", "false"),
            ])
            .send().await.map_err(|e| SwapError::Quote(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(SwapError::Quote(format!("HTTP {}", resp.status())));
        }
        resp.json::<JupQuote>().await.map_err(|e| SwapError::Quote(e.to_string()))
    }

    /// Guard: re-quoted route must be within 20% of locked quote.
    pub fn assert_route_stability(locked_out: &str, refreshed_out: &str) -> Result<(), SwapError> {
        let locked: f64 = locked_out.parse().unwrap_or(0.0);
        let fresh: f64 = refreshed_out.parse().unwrap_or(0.0);
        if locked <= 0.0 || fresh <= 0.0 {
            return Err(SwapError::RouteDeviation { observed_bps: u64::MAX, locked_bps: 2000 });
        }
        let drop_bps = (((locked - fresh) / locked).max(0.0) * 10_000.0) as u64;
        if drop_bps > 2_000 {
            Err(SwapError::RouteDeviation { observed_bps: drop_bps, locked_bps: 2_000 })
        } else { Ok(()) }
    }

    /// Build unsigned serialized swap transaction from Jupiter /swap endpoint.
    pub async fn build_swap_tx(&self, quote: &JupQuote, taker: &str) -> Result<Vec<u8>, SwapError> {
        let body = serde_json::json!({
            "quoteResponse": quote,
            "userPublicKey": taker,
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "prioritizationFeeLamports": { "priorityLevelWithMaxLamports": {
                "maxLamports": 50_000, "priorityLevel": "high" } },
            // Platform fee split configured server-side on the Jupiter fee account:
            "feeAccount": format!("{}/{}", TREASURY, MINT_USDT),
        });

        let resp = self.http.post(format!("{}/swap", self.api_base))
            .json(&body).send().await
            .map_err(|e| SwapError::Broadcast(e.to_string()))?;

        let v: serde_json::Value = resp.json().await
            .map_err(|e| SwapError::Broadcast(e.to_string()))?;
        let b64 = v["swapTransaction"].as_str()
            .ok_or_else(|| SwapError::Broadcast("missing swapTransaction".into()))?;
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.decode(b64)
            .map_err(|e| SwapError::Broadcast(e.to_string()))
    }

    /// Full lifecycle: sign -> send -> poll confirmation. Caller supplies decrypted key bytes.
    pub async fn execute(
        &self,
        signer_key_bytes: &[u8],
        req: &SwapRequest,
    ) -> Result<String, SwapError> {
        let locked_quote = self.quote(req).await?;

        // Radar gate before any signing (Vol Delta §2.3)
        if req.output_mint != MINT_SOL && req.input_mint != MINT_SOL {
            let verdict = crate::goplus::scan_solana_token(&req.output_mint).await
                .map_err(|e| SwapError::RadarBlocked { reason: e.to_string() })?;
            crate::radar::enforce(&verdict)?;
        }

        let raw_tx = self.build_swap_tx(&locked_quote, &req.taker_pubkey).await?;

        let keypair = solana_sdk::signature::Keypair::from_bytes(signer_key_bytes)
            .map_err(|_| SwapError::Broadcast("invalid key material".into()))?;
        let vtx: solana_sdk::transaction::VersionedTransaction =
            bincode::deserialize(&raw_tx)
                .map_err(|e| SwapError::Broadcast(e.to_string()))?;
        let signed = vtx.sign(&[&keypair], None)
            .map_err(|e| SwapError::Broadcast(e.to_string()))?;
        let signature = signed.signatures[0].to_string();

        self.rpc.send_transaction(&signed)
            .await.map_err(|e| SwapError::Broadcast(e.to_string()))?;

        // Confirmation loop: 60s deadline, 2s polls
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            match self.rpc.get_signature_status(&signature) {
                Ok(Some(Ok(()))) => return Ok(signature),
                Ok(Some(Err(_tx_err))) => return Err(SwapError::Broadcast(signature)),
                _ => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
            }
        }
        Err(SwapError::ConfirmTimeout(signature))
    }
}
```

### 3.4 `src/goplus.rs` + `src/radar.rs`

```rust
// goplus.rs
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct TokenSecurity {
    #[serde(rename = "is_honeypot")]       pub is_honeypot: Option<String>,
    #[serde(rename = "sell_tax")]          pub sell_tax: Option<String>,
    #[serde(rename = "buy_tax")]           pub buy_tax: Option<String>,
    #[serde(rename = "is_mintable")]       pub is_mintable: Option<String>,
    #[serde(rename = "owner_change_balance")] pub owner_change_balance: Option<String>,
}

pub async fn scan_solana_token(mint: &str) -> Result<TokenSecurity, String> {
    let url = format!("https://api.gopluslabs.io/api/v1/solana/token_security?contract_addresses={mint}");
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let first = v["result"][0].clone();
    serde_json::from_value(first).map_err(|e| e.to_string())
}
```

```rust
// radar.rs
use crate::goplus::TokenSecurity;
use crate::jupiter::SwapError;

pub fn enforce(t: &TokenSecurity) -> Result<(), SwapError> {
    let honeypot = t.is_honeypot.as_deref() == Some("1");
    let sell_tax: f64 = t.sell_tax.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let buy_tax:  f64 = t.buy_tax.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let mintable = t.is_mintable.as_deref() == Some("1");

    if honeypot {
        return Err(SwapError::RadarBlocked { reason: "honeypot detected".into() });
    }
    if sell_tax > 10.0 {
        return Err(SwapError::RadarBlocked { reason: format!("sell tax {sell_tax}%") });
    }
    if buy_tax > 15.0 {
        return Err(SwapError::RadarBlocked { reason: format!("buy tax {buy_tax}%") });
    }
    if mintable {
        return Err(SwapError::RadarBlocked { reason: "mint authority active".into() });
    }
    Ok(())
}

pub fn requires_user_confirmation(t: &TokenSecurity) -> bool {
    t.owner_change_balance.as_deref() == Some("1")
}
```

### 3.5 `src/p2p.rs` — phone-number transfers

```rust
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum P2pError {
    #[error("recipient not registered on MiddleMan")]
    RecipientNotFound,
    #[error("cannot transfer to self")]
    SelfTransfer,
    #[error("insufficient balance")]
    Insufficient,
    #[error("velocity guard tripped: cool down {minutes}m")]
    VelocityGuard { minutes: u32 },
    #[error("db failure: {0}")]
    Db(String),
}

/// Resolves a +234... phone to user_id; returns None when unregistered.
pub async fn resolve_recipient(
    pool: &sqlx::Pool<sqlx::Postgres>,
    normalized_phone: &str,
) -> Result<Option<Uuid>, P2pError> {
    let row = sqlx::query!(
        r#"SELECT id FROM users WHERE whatsapp_number = $1"#,
        normalized_phone
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| P2pError::Db(e.to_string()))?;
    Ok(row.map(|r| r.id))
}

/// Redis sliding-window velocity check: >3 transfers / 60s => 15 min cooldown.
pub async fn velocity_check(
    conn: &mut impl redis::aio::ConnectionLike,
    sender_id: Uuid,
) -> Result<(), P2pError> {
    let key = format!("velocity:p2p:{sender_id}");
    use redis::AsyncCommands;
    let count: i64 = redis::cmd("INCR")
        .arg(&key)
        .query_async(conn)
        .await
        .map_err(|e| P2pError::Db(e.to_string()))?;
    if count == 1 {
        let _: () = redis::cmd("EXPIRE").arg(&key).arg(60)
            .query_async(conn).await
            .map_err(|e| P2pError::Db(e.to_string()))?;
    }
    if count > 3 {
        let cd = format!("cooldown:{sender_id}");
        let _: () = redis::cmd("SET").arg(&cd).arg(1)
            .arg("EX").arg(900).query_async(conn).await
            .map_err(|e| P2pError::Db(e.to_string()))?;
        return Err(P2pError::VelocityGuard { minutes: 15 });
    }
    Ok(())
}

/// Executes an internal transfer. All ledger mutation is delegated to mm-db's
/// atomic routine (Vol 2 §4.1); this function is orchestration only.
pub async fn execute_internal_transfer(
    state: &crate::NoopStateAlias,
    sender_id: Uuid,
    recipient_phone: &str,
    amount_ngn: Decimal,
) -> Result<Uuid, P2pError> {
    // 1. guards
    if let Some(recipient) = resolve_recipient(&state.db_pool, recipient_phone).await? {
        if recipient == sender_id { return Err(P2pError::SelfTransfer); }
        let mut conn = state.redis.get_multiplexed_async_connection().await
            .map_err(|e| P2pError::Db(e.to_string()))?;
        velocity_check(&mut conn, sender_id).await?;
        let fee = amount_ngn * Decimal::new(5, 3);           // 0.5% platform fee
        crate::db_bridge::p2p_transfer_atomic(&state.db_pool, sender_id, recipient,
                                              amount_ngn, fee).await
            .map_err(P2pError::from_db)
    } else {
        Err(P2pError::RecipientNotFound)
    }
}
```

> `crate::NoopStateAlias` and `db_bridge` are thin re-exports of `mm-api`'s `AppState` and the
> Vol 2 §4.1 routine respectively; they exist so `mm-crypto` compiles without a dependency
> cycle (`mm-api` depends on `mm-crypto`, never the reverse).

## 4. Data Schemas & Structural Interfaces

| Interface | Contract |
|---|---|
| `KeyProvisioner::provision_solana(user_id)` | `(public_address, aead_envelope)`; envelope bound via AAD |
| `JupiterClient::execute(key_bytes, req)` | Returns Solana signature string; errors per §3.3 enum |
| `goplus::scan_solana_token(mint)` | `TokenSecurity` struct (string-typed flags per GoPlus API) |
| `p2p::execute_internal_transfer(...)` | Ledger tx id; all-or-nothing |
| Env | `SOLANA_RPC_URL`, `TREASURY_WALLET`, `YELLOW_CARD_API_KEY` |

Transaction row mapping:

```
DEX_SWAP   metadata: {input_mint, output_mint, in_amount_raw, out_amount_raw, slippage_bps, signature}
P2P_TRANSFER metadata: {counterpart_user, channel: "internal"}
```

## 5. Error Handling, Retry & Edge Cases

| Case | Policy |
|---|---|
| Jupiter quote 404/empty routes | User told pair unsupported; no retry loop |
| Route deviation > 20% | Abort BEFORE signing; re-display fresh quote for confirmation |
| Broadcast ok but confirmation timeout | Mark tx `FAILED_TIMEOUT`; background reconciler checks finality every 10 min ×36; refunds reserved funds only after definitive failure |
| RPC degraded | Fail-closed; swaps disabled via feature flag `swaps_enabled` in Redis config key |
| GoPlus unreachable | For non-stablecoin outputs: require explicit user risk acceptance text "I ACCEPT RISK"; stablecoin pairs bypass radar |
| P2P to unregistered number | Offer on-chain withdrawal alternative with address prompt |
| Key decrypt AuthFailed | Hard stop; tamper alert to ops; account frozen flag |

## 6. Verification Test Cases & Command Sequences

```bash
# VD-T1: provisioning roundtrip (encrypt->decrypt->keypair equality)
cargo test -p mm-crypto keys::

# VD-T2: route stability math
cargo test -p mm-crypto jupiter::tests::route_deviation_bounds

# VD-T3: radar enforcement vectors
cargo test -p mm-crypto radar::
# honeypot=1 blocked; sell_tax=12.5 blocked; owner_change_balance warns only

# VD-T4: devnet swap smoke (gated)
SOLANA_RPC_URL=https://api.devnet.solana.com cargo test -p mm-crypto --ignored devnet_swap_smoke

# VD-T5: P2P atomicity under concurrency
psql $DATABASE_URL -c "SELECT count(*) FROM transactions WHERE tx_type='P2P_TRANSFER';"
# run 20 parallel 10 NGN transfers from a wallet funded with exactly 190 NGN (+fees):
# expect 19 successes... actually expect floor(balance/(amt+fee)) successes and zero negative balances:
psql $DATABASE_URL -c "SELECT min(balance) FROM wallets;"   # must be >= 0

# VD-T6: velocity guard
redis-cli DEL velocity:p2p:<uid>
# fire 5 rapid transfers via FSM stub; 4th/5th must return VelocityGuard message
```
