use crate::state::AppState;
use mm_core::fsm::FlowState;
use mm_db::queries as db;
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum FsmError {
    #[error("retryable failure: {0}")]
    Retry(String),
    #[error("terminal failure: {0}")]
    Terminal(String),
}

impl FsmError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, FsmError::Retry(_))
    }
}

pub struct FsmInput {
    pub message_id: String,
    pub whatsapp_number: String,
    pub text: String,
    pub media_url: Option<String>,
}

const RATE_FLOOR: i64 = 1400;
const MIN_CARD_USD: f64 = 10.0;
const MAX_CARD_USD: f64 = 2000.0;

pub async fn advance(state: &AppState, input: FsmInput) -> Result<(), FsmError> {
    let user = db::ensure_user(&state.pool, &input.whatsapp_number)
        .await
        .map_err(|e| FsmError::Retry(e.to_string()))?;

    let jid = format!("{}@s.whatsapp.net", input.whatsapp_number);

    if let Some(until) = user.pin_locked_until {
        if until > sqlx::types::chrono::Utc::now() {
            crate::outbound::send_text(
                state,
                &jid,
                "🔒 Your PIN vault is temporarily locked after failed attempts.\n\nTry again later.",
            )
            .await
            .ok();
            return Ok(());
        }
    }

    let parsed = state
        .ai
        .extract_intent(&input.text)
        .await
        .map_err(|e| FsmError::Retry(e.to_string()))?;

    match parsed.intent.as_str() {
        "CHECK_BALANCE" => {
            let balance = db::ensure_ngn_wallet(&state.pool, user.id)
                .await
                .map_err(|e| FsmError::Retry(e.to_string()))?;
            reply(state, &jid, &format!("💼 *NGN Wallet*\n\nAvailable balance: ₦{balance}"))
                .await
        }
        "LIQUIDATE_GIFT_CARD" => handle_gift_card(state, &user.id, &jid, &input, &parsed).await,
        "EXECUTE_DEX_SWAP" | "P2P_TRANSFER" | "OPEN_PERP_POSITION" | "TRANSFER_FIAT"
        | "BUY_AIRTIME" | "CHECK_CONTRACT_SECURITY" => {
            reply(
                state,
                &jid,
                &format!(
                    "🛠️ *{}* is coming online in the next rollout.\n\nRight now I can:\n\
                     💵 Liquidate gift cards (send a photo)\n\
                     💼 Check your NGN balance\n\nType *menu* anytime.",
                    parsed.intent.replace('_', " ")
                ),
            )
            .await
        }
        _ => reply(
            state,
            &jid,
            "👋 Welcome to *MiddleMan* — your WhatsApp neo-bank.\n\nI can help you:\n\
             🎁 *Liquidate a gift card* — send a photo of it with the value (e.g. \"Steam $50\")\n\
             💼 *Check balance* — just ask \"balance\"\n\nWhat would you like to do?",
        )
        .await,
    }
}

struct GiftParams {
    brand: String,
    amount: f64,
}

fn extract_gift_params(parsed: &mm_ai::parser::ParsedIntent) -> Option<GiftParams> {
    let brand = parsed.entities.card_brand.as_deref()?.to_uppercase();
    let amount = parsed.entities.amount?;
    Some(GiftParams { brand, amount })
}

async fn handle_gift_card(
    state: &AppState,
    user_id: &Uuid,
    jid: &str,
    input: &FsmInput,
    parsed: &mm_ai::parser::ParsedIntent,
) -> Result<(), FsmError> {
    let Some(media_url) = input.media_url.clone() else {
        return reply(
            state,
            jid,
            "🎁 Send a *clear photo* of the gift card (front + code visible).\n\n\
             Include the value in the caption, e.g. \"$50 Steam card\".",
        )
        .await;
    };

    let Some(params) = extract_gift_params(parsed) else {
        return reply(
            state,
            jid,
            "📸 Photo received. What's the card *brand and value*?\n\nReply like: \"$50 Steam\"",
        )
        .await;
    };

    if !(MIN_CARD_USD..=MAX_CARD_USD).contains(&params.amount) {
        return reply(
            state,
            jid,
            "⚠️ Card values must be between $10 and $2,000. Please confirm the exact value.",
        )
        .await;
    }

    let claimed_usd = Decimal::try_from(params.amount)
        .map_err(|e| FsmError::Terminal(e.to_string()))?;
    let exact = db::catalogue_rate(&state.pool, &params.brand, "US", "PHYSICAL").await;
    let rate = match exact {
        Some(r) => r,
        None => db::catalogue_rate_any_format(&state.pool, &params.brand, "US")
            .await
            .unwrap_or_else(|| Decimal::from(RATE_FLOOR)),
    };
    let payout = (claimed_usd * rate).round_dp(2);

    let trade_id = db::insert_gift_trade(
        &state.pool,
        db::NewTrade {
            user_id: *user_id,
            brand: &params.brand,
            country: "US",
            card_format: "PHYSICAL",
            claimed_usd,
            rate,
            payout,
            code: None,
            image_url: Some(&media_url),
            message_id: &input.message_id,
        },
    )
    .await
    .map_err(|e| FsmError::Retry(e.to_string()))?;

    db::set_state(
        &state.pool,
        *user_id,
        FlowState::AwaitingTransactionData.as_str(),
        serde_json::json!({ "flow": "GIFT_CARD", "trade_id": trade_id }),
    )
    .await
    .map_err(|e| FsmError::Retry(e.to_string()))?;

    reply(
        state,
        jid,
        &format!(
            "*Trade received* 🧾\n\nCard: {}\nValue: ${}\nRate: ₦{}/$\n\n*Payout:* ₦{payout}\n\n\
             Status: PENDING REVIEW — you'll hear from our desk shortly.",
            params.brand, params.amount, rate
        ),
    )
    .await
}

async fn reply(state: &AppState, jid: &str, text: &str) -> Result<(), FsmError> {
    crate::outbound::send_text(state, jid, text)
        .await
        .map_err(|e| FsmError::Terminal(format!("outbound failed: {e}")))
}
