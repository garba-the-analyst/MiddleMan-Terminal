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
    // OTP verification shortcut: if 6-digit code and matches cached OTP, cache PIN and confirm
    if let Some(otp) = crate::security::extract_otp(&input.text) {
        if crate::security::verify_otp(state, &input.whatsapp_number, &otp).await {
            crate::security::cache_pin_ok(state, &input.whatsapp_number).await;
            crate::outbound::send_text(state, &jid, "✅ OTP verified. Your pending transaction is now authorized. Please resend the original command.").await.ok();
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

    // Cross-currency preference: if user says "pay with USDT" or "in USD"
    let _cross_pref = input.text.to_lowercase().contains("usdt") || input.text.to_lowercase().contains("usd") || input.text.to_lowercase().contains("usdc");

    let result: Result<(), FsmError> = match intent {
        "CHECK_BALANCE" => {
            let summary = crate::wallet::wallet_summary(state, user.id).await;
            // also show foreign accounts preview
            let foreign = db::list_foreign_accounts(&state.pool, user.id).await.unwrap_or_default();
            let mut txt = summary;
            if !foreign.is_empty() {
                txt.push_str("\n\n🌍 Foreign accounts:\n");
                for fa in foreign { txt.push_str(&format!("{}: {} ({})\n", fa.currency, fa.account_number, fa.status)); }
                txt.push_str("Reply `create USD account` to open new.");
            }
            reply(state, &jid, &txt).await
        }
        "LIQUIDATE_GIFT_CARD" => handle_gift_card(state, &user.id, &jid, &input, &parsed).await,
        "P2P_TRANSFER" => handle_p2p(state, &user.id, &jid, &input, &parsed).await,
        "TRANSFER_FIAT" => handle_fiat_payout(state, &user.id, &jid, &input, &parsed).await,
        "BUY_AIRTIME" => handle_airtime(state, &user.id, &jid, &input, &parsed).await,
        "SET_PIN" => handle_set_pin(state, &user.id, &jid, &input).await,
        "CREATE_FOREIGN_ACCOUNT" => handle_create_foreign(state, &user.id, &jid, &input).await,
        "EXECUTE_DEX_SWAP" => handle_swap_offramp(state, &user.id, &jid, &input, &parsed).await,
        "OPEN_PERP_POSITION" => handle_perp(state, &user.id, &jid, &parsed).await,
        "CHECK_CONTRACT_SECURITY" => handle_radar(state, &jid, &parsed).await,
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
    input: &FsmInput,
    parsed: &mm_ai::parser::ParsedIntent,
) -> Result<(), FsmError> {
    // PIN/OTP handling: if text contains pin, verify now
    let wa = jid.trim_end_matches("@s.whatsapp.net");
    if let Some(pin) = crate::security::extract_pin(&input.text) {
        let _ = crate::security::require_pin(state, *sender_id, wa, Decimal::from(10_000), &input.text).await;
        // require_pin will cache if valid; if not, it returns Err with message – we let flow continue to amount check
        if let Err(msg) = crate::security::require_pin(state, *sender_id, wa, Decimal::from(10_000), &input.text).await {
            if msg.contains("Wrong PIN") { return reply(state, jid, &msg).await; }
        }
    }
    if let Some(otp) = crate::security::extract_otp(&input.text) {
        if crate::security::verify_otp(state, wa, &otp).await {
            crate::security::cache_pin_ok(state, wa).await;
        }
    }
    let Some(amount_f) = parsed.entities.amount else {
        return reply(state, jid, "💸 How much do you want to send? Example: \"Send 5000 to 08012345678\"").await;
    };
    let Some(recipient_phone) = parsed.entities.recipient_phone.clone() else {
        return reply(state, jid, "📱 Who should I send to? Include a Nigerian number like 08012345678.").await;
    };
    let normalized = mm_ai::normalizer::normalize_text(&recipient_phone);
    let recipient_clean = normalized.split_whitespace().find(|s| s.starts_with("+234")).unwrap_or(&recipient_phone).to_string();
    let recipient_user = db::ensure_user(&state.pool, &recipient_clean.replace('+', "")).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    let _ = db::ensure_ngn_wallet(&state.pool, recipient_user.id).await;
    let amount = Decimal::try_from(amount_f).map_err(|e| FsmError::Terminal(e.to_string()))?;
    let fee = Decimal::ZERO;
    if !db::check_velocity(&state.pool, *sender_id, amount).await.map_err(|e| FsmError::Retry(e.to_string()))? {
        return reply(state, jid, "⚠️ Limit hit: 5 tx/hour or ₦500k/day. Try later or contact support.").await;
    }
    if amount >= Decimal::from(10_000) {
        match crate::security::require_pin(state, *sender_id, wa, amount, &input.text).await {
            Ok(true) => {},
            Ok(false) => return reply(state, jid, "🔐 PIN required").await,
            Err(msg) => return reply(state, jid, &msg).await,
        }
        if amount >= Decimal::from(100_000) && crate::security::extract_otp(&input.text).is_none() {
            // if no OTP provided, generate and ask
            let otp = crate::security::generate_otp(state, wa).await;
            return reply(state, jid, &format!("🔐 Large transfer — OTP {} (mock). Reply 6 digits.", otp)).await;
        }
    }
    let balance = db::ensure_ngn_wallet(&state.pool, *sender_id).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    if balance < amount + fee {
        return reply(state, jid, &format!("❌ Insufficient balance. You have ₦{balance}, trying to send ₦{amount}.")).await;
    }
    match db::p2p_transfer_atomic(&state.pool, *sender_id, recipient_user.id, amount, fee).await {
        Ok(tx_id) => reply(state, jid, &format!("✅ Sent ₦{amount} to {}.\nTx: {}\nNew balance: ₦{}", recipient_clean, &tx_id.to_string()[..8], balance - amount - fee)).await,
        Err(db::DbError::InsufficientFunds) => reply(state, jid, "❌ Insufficient funds.").await,
        Err(e) => Err(FsmError::Retry(e.to_string())),
    }
}

async fn handle_fiat_payout(state: &AppState, user_id: &Uuid, jid: &str, input: &FsmInput, parsed: &mm_ai::parser::ParsedIntent) -> Result<(), FsmError> {
    let amount_f = parsed.entities.amount.ok_or_else(|| FsmError::Terminal("amount missing".into()))?;
    let amount = Decimal::try_from(amount_f).map_err(|e| FsmError::Terminal(e.to_string()))?;
    // extract bank details: 10-digit account + bank code hint
    let bank_code = if input.text.to_lowercase().contains("gtb") { "058" } else if input.text.to_lowercase().contains("access") { "044" } else if input.text.to_lowercase().contains("uba") { "033" } else { "044" };
    let acct = regex::Regex::new(r"\b\d{10}\b").unwrap().find(&input.text).map(|m| m.as_str().to_string()).unwrap_or_else(|| "0123456789".into());
    let wa = jid.trim_end_matches("@s.whatsapp.net");
    if amount >= Decimal::from(10_000) {
        match crate::security::require_pin(state, *user_id, wa, amount, &input.text).await {
            Ok(true) => {},
            Ok(false) => return reply(state, jid, "🔐 PIN required").await,
            Err(msg) => return reply(state, jid, &msg).await,
        }
        if amount >= Decimal::from(100_000) && crate::security::extract_otp(&input.text).is_none() {
            let otp = crate::security::generate_otp(state, wa).await;
            return reply(state, jid, &format!("🔐 Large payout — OTP {} sent (mock). Reply 6 digits to confirm.", otp)).await;
        }
    }
    let bal = db::ensure_ngn_wallet(&state.pool, *user_id).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    if bal < amount { return reply(state, jid, &format!("❌ Balance ₦{} insufficient for ₦{}.", bal, amount)).await; }
    if !db::check_velocity(&state.pool, *user_id, amount).await.map_err(|e| FsmError::Retry(e.to_string()))? {
        return reply(state, jid, "⚠️ Velocity limit.").await;
    }
    let (id, fee) = db::create_fiat_payout(&state.pool, *user_id, amount, bank_code, &acct, None).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    reply(state, jid, &format!("✅ Payout ₦{} → {} ({}) queued. Fee ₦{} (platform). Ref: {}. Bal: ₦{}", amount, acct, bank_code, fee, &id.to_string()[..8], bal - amount - fee)).await
}

async fn handle_airtime(state: &AppState, user_id: &Uuid, jid: &str, _input: &FsmInput, parsed: &mm_ai::parser::ParsedIntent) -> Result<(), FsmError> {
    let amount_f = parsed.entities.amount.unwrap_or(500.0);
    let amount = Decimal::try_from(amount_f).map_err(|e| FsmError::Terminal(e.to_string()))?;
    let network = parsed.entities.network.clone().unwrap_or_else(|| "MTN".into());
    let recipient = parsed.entities.recipient_phone.clone().unwrap_or_else(|| jid.trim_end_matches("@s.whatsapp.net").to_string());
    let bal = db::ensure_ngn_wallet(&state.pool, *user_id).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    if bal < amount { return reply(state, jid, &format!("❌ Balance ₦{} < ₦{}", bal, amount)).await; }
    let (id, fee) = db::create_airtime(&state.pool, *user_id, &recipient, &network, amount, "AIRTIME").await.map_err(|e| FsmError::Retry(e.to_string()))?;
    reply(state, jid, &format!("✅ Airtime ₦{} {} → {}. Fee ₦{}. Ref: {}", amount, network, recipient, fee, &id.to_string()[..8])).await
}

async fn handle_swap_offramp(state: &AppState, user_id: &Uuid, jid: &str, _input: &FsmInput, parsed: &mm_ai::parser::ParsedIntent) -> Result<(), FsmError> {
    let amount_f = parsed.entities.amount.unwrap_or(0.5);
    let amount = Decimal::try_from(amount_f).map_err(|e| FsmError::Terminal(e.to_string()))?;
    let src = parsed.entities.source_currency.clone().unwrap_or_else(|| "SOL".into());
    let dst = parsed.entities.target_currency.clone().unwrap_or_else(|| "NGN".into());
    // crypto -> fiat offramp
    if dst.to_uppercase()=="NGN" {
        let pair = format!("{}/NGN", src.to_uppercase());
        let rate = db::get_crypto_rate(&state.pool, &pair).await.map_err(|e| FsmError::Retry(e.to_string()))?.unwrap_or(Decimal::from(85000));
        let (tx, fee) = db::atomic_offramp(&state.pool, *user_id, &src, "NGN", amount, rate).await.map_err(|e| FsmError::Retry(e.to_string()))?;
        return reply(state, jid, &format!("✅ Offramp {} {} → ₦{} @ ₦{}/{} . Fee ₦{}. Ref: {}", amount, src, (amount*rate).round_dp(2), rate, src, fee, &tx.to_string()[..8])).await;
    }
    // onchain crypto transfer if recipient_address present
    if let Some(to) = parsed.entities.recipient_address.clone() {
        let chain = if to.starts_with("0x") { "EVM" } else { "SOLANA" };
        let (id, hash, fee) = db::create_crypto_transfer(&state.pool, *user_id, chain, &src, amount, &to).await.map_err(|e| FsmError::Retry(e.to_string()))?;
        return reply(state, jid, &format!("✅ Crypto {} {} → {} ({}) . Fee ₦{} + gas. Hash: {} Ref: {}", amount, src, &to[..12.min(to.len())], chain, fee, &hash[..12], &id.to_string()[..8])).await;
    }
    // fiat->crypto onramp
    if src.to_uppercase()=="NGN" {
        let pair = format!("{}/NGN", dst.to_uppercase());
        let rate = db::get_crypto_rate(&state.pool, &pair).await.map_err(|e| FsmError::Retry(e.to_string()))?.unwrap_or(Decimal::from(1600));
        let token_amt = (amount / rate).round_dp(6);
        let (tx, fee) = db::atomic_offramp(&state.pool, *user_id, "NGN", &dst, amount, Decimal::ONE/rate).await.map_err(|e| FsmError::Retry(e.to_string()))?;
        return reply(state, jid, &format!("✅ Onramp ₦{} → {} {} @ ₦{}/{} Fee ₦{} Ref: {}", amount, token_amt, dst, rate, dst, fee, &tx.to_string()[..8])).await;
    }
    let is_degen = ["PEPE","BONK","WIF","MEME","SHIB"].iter().any(|m| src.to_uppercase().contains(m) || dst.to_uppercase().contains(m));
    let fee_type = if is_degen { "DEGEN" } else { "SPOT" };
    let fee = db::platform_fee_for(&state.pool, fee_type, amount).await.unwrap_or(rust_decimal::Decimal::ZERO);
    sqlx::query!("INSERT INTO transactions (user_id, tx_type, direction, amount, currency, fee_amount, status, metadata) VALUES ($1,$2,'OUTBOUND',$3,$4,$5,'SUCCESS', jsonb_build_object('pair',$6::text,'fee_type',$7::text))", user_id, fee_type, amount, src, fee, format!("{}->{}", src, dst), fee_type).execute(&state.pool).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    reply(state, jid, &format!("🔄 Swap {} {} → {} queued ({} via DexScreener/Raydium). Fee {} {} ({}).", amount, src, dst, fee_type, fee, src, fee_type)).await
}

async fn handle_perp(state: &AppState, user_id: &Uuid, jid: &str, parsed: &mm_ai::parser::ParsedIntent) -> Result<(), FsmError> {
    let pair = parsed.entities.source_currency.clone().unwrap_or_else(|| "SOL/USDT".into());
    let amount = Decimal::try_from(parsed.entities.amount.unwrap_or(100.0)).unwrap_or(Decimal::from(100));
    // degen: use DexScreener mock trending check
    let fee = db::platform_fee_for(&state.pool, "FUTURES", amount).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    let row = sqlx::query!("INSERT INTO active_positions (user_id, protocol, market_pair, side, leverage, margin_usd, entry_price, liquidation_price, status) VALUES ($1,'raydium',$2,'LONG',5,$3,100,90,'OPEN') RETURNING id", user_id, pair, amount).fetch_one(&state.pool).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    sqlx::query!("INSERT INTO transactions (user_id, tx_type, direction, amount, currency, fee_amount, status, metadata) VALUES ($1,'FUTURES','OUTBOUND',$2,'USD',$3,'SUCCESS', jsonb_build_object('pair',$4::text,'fee',$5::text))", user_id, amount, fee, pair, fee.to_string()).execute(&state.pool).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    reply(state, jid, &format!("📈 Perp {} x5 opened. Margin ${} fee ${} (1%). Liq 90. ID: {}.", pair, amount, fee, &row.id.to_string()[..8])).await
}
async fn handle_set_pin(state: &AppState, user_id: &Uuid, jid: &str, input: &FsmInput) -> Result<(), FsmError> {
    if let Some(pin) = crate::security::extract_pin(&input.text) {
        if pin.len() <4 || pin.len()>6 { return reply(state, jid, "PIN must be 4-6 digits. Send `pin 1234`.").await; }
        let hash = mm_vault::pin_hash::hash_password(&pin).map_err(|e| FsmError::Terminal(e.to_string()))?;
        db::set_user_pin(&state.pool, *user_id, &hash).await.map_err(|e| FsmError::Retry(e.to_string()))?;
        crate::security::cache_pin_ok(state, jid.trim_end_matches("@s.whatsapp.net")).await;
        return reply(state, jid, "✅ PIN set. Cached 15 min. Now retry your transaction with PIN if needed.").await;
    }
    reply(state, jid, "🔐 Send your new PIN: `pin 1234` (4-6 digits).").await
}
async fn handle_create_foreign(state: &AppState, user_id: &Uuid, jid: &str, input: &FsmInput) -> Result<(), FsmError> {
    let cur = if input.text.to_lowercase().contains("gbp") { "GBP" } else if input.text.to_lowercase().contains("eur") { "EUR" } else { "USD" };
    let acct = db::ensure_foreign_wallet(&state.pool, *user_id, cur).await.map_err(|e| FsmError::Retry(e.to_string()))?;
    let wallets = db::list_wallets(&state.pool, *user_id).await.unwrap_or_default();
    let bal = wallets.iter().find(|w| w.currency==cur).map(|w| w.balance).unwrap_or(Decimal::ZERO);
    reply(state, jid, &format!("🌍 {} account {} created (mock). Balance {} {}. For freelancers: share this account with overseas clients. Future: Wise/Stripe provider.", cur, acct, bal, cur)).await
}

async fn reply(state: &AppState, jid: &str, text: &str) -> Result<(), FsmError> {
    crate::outbound::send_text(state, jid, text)
        .await
        .map_err(|e| FsmError::Terminal(format!("outbound failed: {e}")))
}
