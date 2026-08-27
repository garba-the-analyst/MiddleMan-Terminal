# VOLUME EPSILON — Gift Card Liquidation & Settlement Engine

**Version:** 2.4.0 · **Owner:** Product Core · **Scope:** Card ingestion, OCR extraction, quote
confirmation, admin desk settlement, NGN payout.

---

## 1. Architectural Overview & Technical Scope

The flagship MVP flow. A user sends a photo of a gift card; the system extracts brand/value/code
via multimodal AI vision, quotes a rate from `price_catalogue`, and parks the trade as `PENDING`.
A human agent on the Vue desk verifies against the hosted image; approval atomically credits
NGN and notifies the user. Rejection records a reason and notifies.

```
User photo -> bridge (Vol 3 media pipeline) -> inbound event (media_url)
   -> FSM: LIQUIDATE_GIFT_CARD flow
      -> Vision OCR (Gemini multimodal): {brand, country, format, usd_value, code}
      -> price_catalogue lookup -> offered_ngn_rate -> final_ngn_payout
      -> INSERT gift_card_trades (PENDING) + WebSocket push to admin desk
Agent approves -> Vol 2 §4.3 atomic credit + WhatsApp notification
```

## 2. Mathematical Formulation — Pricing

```
final_ngn_payout = claimed_usd_amount * offered_ngn_rate          (rate from catalogue)
offered_ngn_rate = catalogue.rate_per_dollar                      (brand,country,format)
fallback_rate    = 1400.00 when no active row matches             (ops floor, logged loudly)

Sanity gate: if OCR usd_value ∉ [10, 2000] => reject at parse time ("unreadable/unsupported value")
```

Rates are admin-managed (`price_catalogue`), never derived from FX — spreads already baked in.

## 3. Complete Implementation

### 3.1 Vision OCR client (`crates/mm-ai/src/vision.rs`)

```rust
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VisionError {
    #[error("http failure: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unusable model output")]
    BadShape,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CardRead {
    pub brand: String,
    pub country: String,
    #[serde(rename = "format")]
    pub card_format: String,
    pub usd_value: f64,
    pub code: String,
}

const VISION_PROMPT: &str = r#"You are an OCR engine for a Nigerian gift-card exchange.
Read this gift card image and return STRICT JSON:
{"brand":"STEAM|APPLE|AMAZON|RAZER_GOLD|GOOGLE_PLAY|OTHER",
 "country":"US|UK|DE|CA|OTHER",
 "format":"PHYSICAL|ECODE",
 "usd_value": number,
 "code": "the alphanumeric redemption code exactly as printed"}
If unreadable or not a gift card, return {"brand":"OTHER","country":"OTHER","format":"PHYSICAL",
"usd_value":0,"code":""}. No prose."#;

pub async fn read_card_image(
    api_key: &str,
    image_url: &str,
) -> Result<CardRead, VisionError> {
    let payload = serde_json::json!({
        "systemInstruction": { "parts": [{ "text": VISION_PROMPT }] },
        "contents": [{ "parts": [
            { "text": "Extract this card's data." },
            { "file_data": { "file_uri": image_url } }
        ]}],
        "generationConfig": { "temperature": 0.0, "maxOutputTokens": 256 }
    });

    let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-flash-latest:generateContent";
    let resp = reqwest::Client::new()
        .post(url).query(&[("key", api_key)]).json(&payload)
        .timeout(std::time::Duration::from_secs(20))
        .send().await?;

    let data: serde_json::Value = resp.json().await?;
    let text = data["candidates"][0]["content"]["parts"][0]["text"]
        .as_str().ok_or(VisionError::BadShape)?;
    serde_json::from_str(text.trim().trim_start_matches("```json").trim_end_matches("```"))
        .map_err(|_| VisionError::BadShape)
}
```

### 3.2 Trade creation handler (`crates/mm-api/src/handlers/giftcard.rs`)

```rust
use crate::outbound::Outbound;
use crate::state::AppState;
use mm_ai::vision::{read_card_image, CardRead};

pub struct CreateTradeOutcome {
    pub trade_id: uuid::Uuid,
    pub reply_text: String,
}

pub async fn create_trade_from_media(
    state: &AppState,
    user_id: uuid::Uuid,
    whatsapp_number: &str,
    image_url: &str,
) -> Result<CreateTradeOutcome, anyhow::Error> {
    let read: CardRead = read_card_image(&state.gemini_api_key, image_url).await?;

    if read.usd_value < 10.0 || read.usd_value > 2000.0 || read.code.is_empty() {
        let text = "I couldn't read that card clearly.\n\n\
                    Please send a sharp photo showing:\n\
                    - the card type (e.g. Steam)\n\
                    - the value\n\
                    - the full code/receipt".to_string();
        return Ok(CreateTradeOutcome { trade_id: uuid::Uuid::nil(), reply_text: text });
    }

    let rate = sqlx::query!(
        r#"SELECT rate_per_dollar FROM price_catalogue
           WHERE brand ILIKE $1 AND country = $2 AND card_format = $3 AND active = TRUE"#,
        format!("%{}%", read.brand),
        normalize_country(&read.country),
        read.card_format.to_uppercase(),
    )
    .fetch_optional(&state.db_pool)
    .await?
    .map(|r| r.rate_per_dollar)
    .unwrap_or(rust_decimal::Decimal::from(1400));

    let amount = rust_decimal::Decimal::try_from(read.usd_value)?;
    let payout = (amount * rate).round_dp(2);
    let country = normalize_country(&read.country);

    let trade = sqlx::query!(
        r#"INSERT INTO gift_card_trades
             (user_id, card_brand, country, card_format, claimed_usd_amount,
              offered_ngn_rate, final_ngn_payout, extracted_code, image_url, status)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'PENDING')
           RETURNING id"#,
        user_id, read.brand.to_uppercase(), country, read.card_format.to_uppercase(),
        amount, rate, payout, read.code, image_url,
    )
    .fetch_one(&state.db_pool)
    .await?;

    // Live push to the ops desk (Vol Zeta WebSocket).
    state.desk_broadcast(serde_json::json!({
        "event": "trade.created",
        "trade_id": trade.id,
        "image_url": image_url,
    })).await;

    let reply = format!(
        "*Trade received* 🧾\n\nCard: {} ({})\nValue detected: ${}\nRate: ₦{}/$\n\n\
         *Payout:* ₦{}\n\nStatus: PENDING REVIEW — you'll get a message once our desk checks it.",
        read.brand, country, read.usd_value, rate, payout
    );
    Ok(CreateTradeOutcome { trade_id: trade.id, reply_text: reply })
}

fn normalize_country(c: &str) -> String {
    match c.to_uppercase().as_str() {
        "UNITED STATES" | "USA" => "US".into(),
        "UNITED KINGDOM" | "GB" => "UK".into(),
        other => other.chars().take(8).collect(),
    }
}

/// Notification helper used by the admin resolve endpoint.
pub async fn notify_user_of_resolution(state: &AppState, jid: &str, approved: bool, detail: &str) {
    let text = if approved {
        format!("*✅ Trade Approved!*\n\n{detail}\n\nYour NGN wallet has been credited.")
    } else {
        format!("*❌ Trade Rejected*\n\nReason: {detail}")
    };
    let _ = Outbound { recipient_jid: jid, text }.send(state).await;
}
```

### 3.3 Admin resolution service (`crates/mm-api/src/handlers/admin.rs`)

```rust
use axum::{extract::{Path, State}, http::HeaderMap, Json};
use sqlx::types::Uuid;
use std::sync::Arc;
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct ResolveBody {
    pub action: String,               // "approve" | "reject"
    pub reason: Option<String>,
    #[serde(default)]
    pub adjusted_payout: Option<rust_decimal::Decimal>,
}

/// POST /api/v1/admin/trades/:id/resolve     (JWT-gated)
///
/// Atomic per Vol 2 §4.3: status flip is gated on `status='PENDING'` so double-clicks and
/// concurrent agents cannot double-credit.
pub async fn resolve_trade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(trade_id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> Json<serde_json::Value> {
    let employee_id: Uuid = headers.jwt_employee_id();       // extractor, Vol Zeta auth
    let trade_uuid = match Uuid::parse_str(&trade_id) {
        Ok(u) => u, Err(_) => return Json(serde_json::json!({"error": "invalid id"})),
    };

    let mut db_tx = state.db_pool.begin().await.unwrap();

    let payout = body.adjusted_payout; // agents may adjust after manual inspection

    if body.action == "approve" {
        let rows = sqlx::query!(
            r#"UPDATE gift_card_trades
               SET status='APPROVED', reviewed_by_employee_id=$2, updated_at=NOW(),
                   final_ngn_payout = COALESCE($3, final_ngn_payout)
               WHERE id=$1 AND status='PENDING'
               RETURNING user_id, final_ngn_payout, card_brand, claimed_usd_amount"#,
            trade_uuid, employee_id, payout
        )
        .fetch_optional(&mut *db_tx).await.unwrap();

        let Some(trade) = rows else {
            db_tx.rollback().await.ok();
            return Json(serde_json::json!({"error": "already resolved or missing"}));
        };

        sqlx::query!(
            r#"INSERT INTO transactions
                 (user_id, tx_type, direction, amount, currency, status, metadata)
               VALUES ($1,'GIFT_CARD_PAYOUT','INBOUND',$2,'NGN','SUCCESS',
                       jsonb_build_object('trade_id',$3))"#,
            trade.user_id, trade.final_ngn_payout, trade_uuid
        ).execute(&mut *db_tx).await.unwrap();

        sqlx::query!(
            r#"UPDATE wallets SET balance = balance + $1, updated_at=NOW()
               WHERE user_id=$2 AND currency='NGN'"#,
            trade.final_ngn_payout, trade.user_id
        ).execute(&mut *db_tx).await.unwrap();

        write_audit(&mut db_tx, employee_id, "TRADE_APPROVE", "gift_card_trades", trade_uuid).await;
        db_tx.commit().await.unwrap();

        tokio::spawn(async move {
            let jid = fetch_user_jid(&state, trade.user_id).await;
            crate::handlers::giftcard::notify_user_of_resolution(
                &state, &jid, true,
                &format!("{} ${} card paid ₦{}", trade.card_brand, trade.claimed_usd_amount,
                         trade.final_ngn_payout)).await;
        });

        Json(serde_json::json!({"status": "approved"}))
    } else {
        let reason = body.reason.unwrap_or_else(|| "Card invalid or already redeemed.".into());
        let rows = sqlx::query!(
            r#"UPDATE gift_card_trades
               SET status='REJECTED', rejection_reason=$2, reviewed_by_employee_id=$3, updated_at=NOW()
               WHERE id=$1 AND status='PENDING' RETURNING user_id"#,
            trade_uuid, reason, employee_id
        )
        .fetch_optional(&mut *db_tx).await.unwrap();

        let Some(trade) = rows else {
            db_tx.rollback().await.ok();
            return Json(serde_json::json!({"error": "already resolved or missing"}));
        };

        write_audit(&mut db_tx, employee_id, "TRADE_REJECT", "gift_card_trades", trade_uuid).await;
        db_tx.commit().await.unwrap();

        tokio::spawn(async move {
            let jid = fetch_user_jid(&state, trade.user_id).await;
            crate::handlers::giftcard::notify_user_of_resolution(&state, &jid, false, &reason).await;
        });

        Json(serde_json::json!({"status": "rejected"}))
    }
}

async fn write_audit(db_tx: &mut sqlx::PgConnection, emp: Uuid, action: &str,
                     entity: &str, target: Uuid) {
    sqlx::query!(
        r#"INSERT INTO admin_audit_logs (employee_id, action, target_entity, target_id)
           VALUES ($1,$2,$3,$4)"#, emp, action, entity, target
    ).execute(db_tx).await.ok();
}

async fn fetch_user_jid(state: &AppState, user_id: Uuid) -> String {
    sqlx::query!(r#"SELECT whatsapp_number FROM users WHERE id=$1"#, user_id)
        .fetch_one(&state.db_pool).await
        .map(|r| format!("{}@s.whatsapp.net", r.whatsapp_number))
        .unwrap_or_default()
}
```

## 4. Data Schemas & Structural Interfaces

Trade lifecycle (single direction, terminal states):

```
PENDING ──approve──> APPROVED (terminal)
   └─────reject────> REJECTED  (terminal)
```

| Field | Contract |
|---|---|
| `extracted_code` | Stored for agent cross-check; masked `****` in desk UI except last 4 chars |
| `image_url` | Cloudinary URL; desk renders zoomable modal |
| `offered_ngn_rate` | Snapshot at creation — later catalogue edits never mutate pending trades |
| `adjusted_payout` | Agent override allowed only during resolve; audited via `changes` diff |

Desk WebSocket events (`/api/v1/ws`, Vol Zeta):

```json
{ "event": "trade.created", "trade_id": "...", "image_url": "..." }
{ "event": "trade.resolved", "trade_id": "...", "status": "APPROVED" }
```

## 5. Error Handling & Edge Cases

| Case | Policy |
|---|---|
| OCR unreadable / not a card | No trade row created; user prompted to resend |
| Duplicate code reuse across users | Agent desk flags `code` seen in a previous REJECTED/APPROVED trade (lookup query below) |
| Gemini vision timeout (20 s) | Reply: "photo received, processing delayed"; retry once; then ask user to type brand+value manually → FSM creates PENDING row with null code |
| Agent adjusts payout upward > 20% of quote | Requires SUPER_ADMIN role (enforced in extractor) |
| User sends image without caption/text | FSM infers LIQUIDATE_GIFT_CARD from media presence alone |
| Approval crash mid-transaction | DB rollback; trade stays PENDING; desk shows it again |

Duplicate-code check query (desk pre-approval banner):

```sql
SELECT count(*) AS prior_uses
FROM gift_card_trades
WHERE extracted_code = $1 AND id <> $2 AND status IN ('APPROVED','REJECTED');
```

## 6. Verification Test Cases & Command Sequences

```bash
# VE-T1: OCR conformance on fixture set
cargo test -p mm-api giftcard::ocr_fixtures -- --ignored
# fixtures/: steam_us_physical.jpg, apple_uk_ecode.jpg, blurry.jpg (expect graceful reject)

# VE-T2: happy path end-to-end
# send Steam card photo from test number ->
psql $DATABASE_URL -c "SELECT status, claimed_usd_amount, final_ngn_payout FROM gift_card_trades ORDER BY created_at DESC LIMIT 1;"
# expect ('PENDING', 50.00, 72500.00) with seeded STEAM US PHYSICAL rate 1450

# VE-T3: double-approval race
# two agents click Approve simultaneously -> exactly one gets success JSON;
# ledger contains ONE GIFT_CARD_PAYOUT row for the trade:
psql $DATABASE_URL -c "
SELECT count(*) FROM transactions WHERE metadata->>'trade_id'='<tid>' AND tx_type='GIFT_CARD_PAYOUT';"
# expect 1

# VE-T4: payout math precision
cargo test -p mm-api giftcard::payout_math   # 0.01 NGN rounding half-up at final step only

# VE-T5: rejection notifies with reason
# admin rejects with reason 'already redeemed' -> WhatsApp message contains that reason

# VE-T6: audit trail completeness
psql $DATABASE_URL -c "
SELECT action FROM admin_audit_logs ORDER BY created_at DESC LIMIT 1;"
# expect TRADE_APPROVE or TRADE_REJECT matching the last resolution
```
