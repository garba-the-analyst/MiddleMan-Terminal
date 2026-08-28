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
    let t0 = std::time::Instant::now();
    let user = db::ensure_user(&state.pool, &input.whatsapp_number)
        .await
        .map_err(|e| FsmError::Retry(e.to_string()))?;

    // Provision wallets on first contact (NGN + Solana/EVM)
    let _ = crate::wallet::ensure_wallets(state, user.id).await;

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

    // === Case Study 1: sentiment + urgency + category classification ===
    let su = mm_ai::sentiment::classify_sentiment_urgency(&input.text, &parsed.intent, parsed.confidence);

    // Knowledge base retrieval for FAQ / unknown
    let kb_hit = if matches!(parsed.intent.as_str(), "UNKNOWN" | "HELP") {
        mm_ai::knowledge::search_kb(&input.text, Some(&su.category))
    } else { None };

    // A photo is an implicit liquidation request — Vision OCR decides the details,
    // so we never bounce an image to the help menu.
    let intent = if input.media_url.is_some()
        && matches!(parsed.intent.as_str(), "UNKNOWN" | "HELP" | "REGISTER_USER")
    {
        "LIQUIDATE_GIFT_CARD"
    } else {
        parsed.intent.as_str()
    };

    // Capture response text for logging before sending
    let result: Result<(), FsmError> = match intent {
        "CHECK_BALANCE" => {
            let summary = crate::wallet::wallet_summary(state, user.id).await;
            reply(state, &jid, &summary).await
        }
        "LIQUIDATE_GIFT_CARD" => handle_gift_card(state, &user.id, &jid, &input, &parsed).await,
        "P2P_TRANSFER" => handle_p2p(state, &user.id, &jid, &parsed).await,
        "CHECK_CONTRACT_SECURITY" => handle_radar(state, &jid, &parsed).await,
        "EXECUTE_DEX_SWAP" | "OPEN_PERP_POSITION" | "TRANSFER_FIAT" | "BUY_AIRTIME" => {
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
        _ => {
            // FAQ / knowledge base path — meets Case Study 1 "Retrieve answers from KB"
            if let Some(ref art) = kb_hit {
                reply(state, &jid, &format!("{}\n\n_Source: {}_", art.answer, art.category)).await
            } else {
                reply(
                    state,
                    &jid,
                    "👋 Welcome to *MiddleMan* — your WhatsApp neo-bank.\n\nI can help you:\n\
                     🎁 *Liquidate a gift card* — send a photo of it with the value (e.g. \"Steam $50\")\n\
                     💼 *Check balance* — just ask \"balance\"\n\
                     💸 *Send money* — \"Send 5000 to 08012345678\"\n\
                     🛡️ *Scan a token* — send a contract address\n\nWhat would you like to do?",
                )
                .await
            }
        }
    };

    let handling_ms = t0.elapsed().as_millis() as i32;
    let response_snippet: Option<String> = None; // we capture inside reply; for now store kb answer or intent
    let resp_text = kb_hit.as_ref().map(|a| a.answer.clone()).or_else(|| Some(format!("handled:{}", intent)));

    // Escalation handling — if flagged, tag interaction and notify (log)
    let escalated = su.escalation;
    if escalated {
        eprintln!("ESCALATE {} cat={} urgency={} sentiment={} reason={:?}", input.whatsapp_number, su.category, su.urgency, su.sentiment, su.escalation_reason);
    }

    // Log interaction for analytics (never fails the main flow)
    let _ = db::insert_bot_interaction(
        &state.pool,
        &input.message_id,
        &input.whatsapp_number,
        Some(user.id),
        &input.text,
        &parsed.intent,
        &su.category,
        &su.sentiment,
        &su.urgency,
        su.urgency_score,
        parsed.confidence,
        resp_text.as_deref(),
        None,
        escalated,
        su.escalation_reason.as_deref(),
        handling_ms,
    ).await;

    // Metrics for dashboard
    let _ = db::upsert_bot_analytics(&state.pool, "messages_inbound", 1, None).await;
    if escalated { let _ = db::upsert_bot_analytics(&state.pool, "escalated", 1, None).await; }
    if kb_hit.is_some() { let _ = db::upsert_bot_analytics(&state.pool, "knowledge_base_hits", 1, None).await; }

    result
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

    // AI Vision OCR first — reads brand, value, country, format and code straight off the image.
    let ocr = match state.cfg.gemini_api_key.as_deref() {
        Some(key) => match mm_ai::vision::read_card_image(key, &media_url).await {
            Ok(read) if read.is_usable() => {
                println!(
                    "vision ocr: {} {} ${} code={}",
                    read.brand, read.country, read.usd_value, !read.code.is_empty()
                );
                Some(read)
            }
            Ok(_) => {
                println!("vision ocr: unreadable card");
                None
            }
            Err(e) => {
                eprintln!("vision ocr failed: {e}");
                None
            }
        },
        None => None,
    };

    // Fall back to caption entities when OCR is unavailable or unusable.
    let (brand, amount_usd, country, card_format, code) = match &ocr {
        Some(r) => (
            r.brand.to_uppercase(),
            r.usd_value,
            normalize_country(&r.country),
            if r.card_format.to_uppercase() == "ECODE" { "ECODE" } else { "PHYSICAL" },
            (!r.code.is_empty()).then(|| r.code.clone()),
        ),
        None => match extract_gift_params(parsed) {
            Some(p) => (p.brand, p.amount, "US".to_string(), "PHYSICAL", None),
            None => {
                return reply(
                    state,
                    jid,
                    "📸 Photo received but I couldn't read the card clearly.\n\n\
                     Reply with the *brand and value*, e.g. \"$50 Steam\".",
                )
                .await;
            }
        },
    };

    if !(MIN_CARD_USD..=MAX_CARD_USD).contains(&amount_usd) {
        return reply(
            state,
            jid,
            "⚠️ Card values must be between $10 and $2,000. Please confirm the exact value.",
        )
        .await;
    }

    let claimed_usd =
        Decimal::try_from(amount_usd).map_err(|e| FsmError::Terminal(e.to_string()))?;
    let exact = db::catalogue_rate(&state.pool, &brand, &country, card_format).await;
    let rate = match exact {
        Some(r) => r,
        None => db::catalogue_rate_any_format(&state.pool, &brand, &country)
            .await
            .unwrap_or_else(|| Decimal::from(RATE_FLOOR)),
    };
    let payout = (claimed_usd * rate).round_dp(2);

    let trade_id = db::insert_gift_trade(
        &state.pool,
        db::NewTrade {
            user_id: *user_id,
            brand: &brand,
            country: &country,
            card_format,
            claimed_usd,
            rate,
            payout,
            code: code.as_deref(),
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

    let detection_line = if ocr.is_some() {
        "🤖 Read automatically from your photo"
    } else {
        "✍️ Based on the details you sent"
    };

    reply(
        state,
        jid,
        &format!(
            "*Trade received* 🧾\n\n{detection_line}\n\nCard: {brand} ({country}, {card_format})\n\
             Value: ${amount_usd}\nRate: ₦{rate}/$\n\n*Payout:* ₦{payout}\n\n\
             Status: PENDING REVIEW — you'll hear from our desk shortly."
        ),
    )
    .await
}

fn normalize_country(raw: &str) -> String {
    match raw.to_uppercase().as_str() {
        "UNITED STATES" | "USA" | "US" => "US".into(),
        "UNITED KINGDOM" | "GB" | "UK" => "UK".into(),
        "GERMANY" | "DE" => "DE".into(),
        "CANADA" | "CA" => "CA".into(),
        _ => "US".into(),
    }
}

async fn handle_radar(
    state: &AppState,
    jid: &str,
    parsed: &mm_ai::parser::ParsedIntent,
) -> Result<(), FsmError> {
    let Some(address) = parsed.entities.contract_address.clone() else {
        return reply(
            state,
            jid,
            "🛡️ Send me a token contract address to scan.\n\nExample: \"check 0x7a250d... for honeypot\"",
        )
        .await;
    };

    match mm_ai::radar::scan_token(&address).await {
        Ok(security) => {
            let verdict = mm_ai::radar::enforce(&security);
            let report = mm_ai::radar::format_report(&address, &security, &verdict);
            reply(state, jid, &report).await
        }
        Err(e) => {
            eprintln!("radar scan failed: {e}");
            reply(
                state,
                jid,
                "🛡️ I couldn't find that token on the security registry.\n\n\
                 Double-check the contract address — unlisted tokens are high risk by default.",
            )
            .await
        }
    }
}

async fn handle_p2p(
    state: &AppState,
    sender_id: &Uuid,
    jid: &str,
    parsed: &mm_ai::parser::ParsedIntent,
) -> Result<(), FsmError> {
    let Some(amount_f) = parsed.entities.amount else {
        return reply(state, jid, "💸 How much do you want to send? Example: \"Send 5000 to 08012345678\"").await;
    };
    let Some(recipient_phone) = parsed.entities.recipient_phone.clone() else {
        return reply(state, jid, "📱 Who should I send to? Include a Nigerian number like 08012345678.").await;
    };

    // Normalize recipient (ensure +234)
    let normalized = mm_ai::normalizer::normalize_text(&recipient_phone);
    let recipient_clean = normalized
        .split_whitespace()
        .find(|s| s.starts_with("+234"))
        .unwrap_or(&recipient_phone)
        .to_string();

    // Ensure recipient exists (auto-create for demo)
    let recipient_user = db::ensure_user(&state.pool, &recipient_clean.replace('+', ""))
        .await
        .map_err(|e| FsmError::Retry(e.to_string()))?;
    let _ = db::ensure_ngn_wallet(&state.pool, recipient_user.id).await;

    let amount = Decimal::try_from(amount_f).map_err(|e| FsmError::Terminal(e.to_string()))?;
    let fee = Decimal::ZERO; // demo: no fee

    // Check balance
    let balance = db::ensure_ngn_wallet(&state.pool, *sender_id)
        .await
        .map_err(|e| FsmError::Retry(e.to_string()))?;
    if balance < amount + fee {
        return reply(
            state,
            jid,
            &format!("❌ Insufficient balance. You have ₦{balance}, trying to send ₦{amount}."),
        )
        .await;
    }

    match db::p2p_transfer_atomic(&state.pool, *sender_id, recipient_user.id, amount, fee).await {
        Ok(tx_id) => {
            reply(
                state,
                jid,
                &format!(
                    "✅ Sent ₦{amount} to {}.\n\nTx: {}\nYour new balance: ₦{}",
                    recipient_clean,
                    &tx_id.to_string()[..8],
                    balance - amount - fee
                ),
            )
            .await
        }
        Err(db::DbError::InsufficientFunds) => {
            reply(state, jid, "❌ Insufficient funds for this transfer.").await
        }
        Err(e) => Err(FsmError::Retry(e.to_string())),
    }
}

async fn reply(state: &AppState, jid: &str, text: &str) -> Result<(), FsmError> {
    crate::outbound::send_text(state, jid, text)
        .await
        .map_err(|e| FsmError::Terminal(format!("outbound failed: {e}")))
}
