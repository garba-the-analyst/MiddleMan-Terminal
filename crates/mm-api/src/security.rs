use crate::state::AppState;
use uuid::Uuid;

const PIN_OK_TTL: usize = 900; // 15 min
const OTP_TTL: usize = 120; // 2 min

pub fn extract_pin(text: &str) -> Option<String> {
    let t = text.trim();
    // "pin 1234" or "1234" alone if 4-6 digits and message is short
    if let Some(caps) = regex::Regex::new(r"(?i)pin\s*(\d{4,6})").unwrap().captures(t) {
        return Some(caps[1].to_string());
    }
    if t.len()>=4 && t.len()<=6 && t.chars().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    None
}

pub async fn is_pin_ok_cached(state: &AppState, wa: &str) -> bool {
    let mut conn = match state.redis.get_multiplexed_async_connection().await { Ok(c)=>c, Err(_)=> return false };
    let v: Option<String> = redis::cmd("GET").arg(format!("pin_ok:{}", wa)).query_async(&mut conn).await.unwrap_or(None);
    v.is_some()
}
pub async fn cache_pin_ok(state: &AppState, wa: &str) {
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        let _: () = redis::cmd("SET").arg(format!("pin_ok:{}", wa)).arg("1").arg("EX").arg(PIN_OK_TTL).query_async(&mut conn).await.unwrap_or(());
    }
}
pub async fn require_pin(state: &AppState, user_id: Uuid, wa: &str, amount_ngn: rust_decimal::Decimal, text: &str) -> Result<bool, String> {
    // thresholds: <10k no PIN, 10k-100k PIN, >100k PIN+OTP (we handle OTP elsewhere)
    // if cached, OK
    if amount_ngn < rust_decimal::Decimal::from(10_000) { return Ok(true); }
    if is_pin_ok_cached(state, wa).await { return Ok(true); }
    // try extract PIN from current text
    if let Some(pin) = extract_pin(text) {
        let hash = crate::state::AppState::get_user_pin_hash(state, user_id).await; // helper below
        if let Some(stored) = hash {
            if mm_vault::pin_hash::verify_pin(&pin, &stored).unwrap_or(false) {
                crate::state::AppState::reset_pin_attempts(state, user_id).await;
                cache_pin_ok(state, wa).await;
                return Ok(true);
            } else {
                let _ = mm_db::queries::increment_pin_fail(&state.pool, user_id).await;
                return Err("❌ Wrong PIN. 3 fails → 2-min lock, 5 fails → 15-min lock. Try again.".into());
            }
        } else {
            // first time PIN set: treat provided digits as new PIN if 4-6 digits
            if pin.len()>=4 && pin.len()<=6 {
                let new_hash = mm_vault::pin_hash::hash_password(&pin).map_err(|e| e.to_string())?;
                let _ = mm_db::queries::set_user_pin(&state.pool, user_id, &new_hash).await;
                cache_pin_ok(state, wa).await;
                return Ok(true);
            }
        }
    }
    Err(format!("🔐 Enter your 4-6 digit PIN to confirm ₦{}.\nSend `pin 1234` — cached 15 min.", amount_ngn))
}

pub async fn generate_otp(state: &AppState, wa: &str) -> String {
    let code = format!("{:06}", rand::random::<u32>() % 1_000_000);
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        let _: () = redis::cmd("SET").arg(format!("otp:{}", wa)).arg(&code).arg("EX").arg(OTP_TTL).query_async(&mut conn).await.unwrap_or(());
    }
    // also store in pg for audit
    let _ = sqlx::query!("INSERT INTO otp_codes (whatsapp_number, code, expires_at) VALUES ($1,$2,NOW()+interval '2 minutes')", wa, code).execute(&state.pool).await;
    code
}
pub async fn verify_otp(state: &AppState, wa: &str, input: &str) -> bool {
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        let v: Option<String> = redis::cmd("GET").arg(format!("otp:{}", wa)).query_async(&mut conn).await.unwrap_or(None);
        if v.as_deref() == Some(input.trim()) {
            let _: () = redis::cmd("DEL").arg(format!("otp:{}", wa)).query_async(&mut conn).await.unwrap_or(());
            return true;
        }
    }
    false
}
pub fn extract_otp(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"\b(\d{6})\b").unwrap();
    re.captures(text).map(|c| c[1].to_string())
}
