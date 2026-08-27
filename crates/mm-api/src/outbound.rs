use crate::state::AppState;
use rand::Rng;
use serde_json::json;

const OUTBOUND_STREAM: &str = "outbound:wa:messages";

pub fn typing_delay_ms(text_len: usize) -> u64 {
    let jitter: i64 = rand::thread_rng().gen_range(-200..=300);
    let raw = text_len as i64 * 35 + jitter;
    raw.clamp(1200, 2500) as u64
}

/// Primary path: HTTP push to the bridge (typing-simulated there).
/// Fallback: Redis stream, drained by the bridge consumer when HTTP is down.
pub async fn send_text(state: &AppState, recipient_jid: &str, text: &str) -> anyhow::Result<()> {
    let body = json!({
        "recipient_jid": recipient_jid,
        "text": text,
        "typing_delay_ms": typing_delay_ms(text.len()),
    });

    let attempt = state
        .http
        .post(format!("{}/bridge/send-message", state.cfg.wa_bridge_url))
        .header("X-Internal-Secret", &state.cfg.internal_api_secret)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match attempt {
        Ok(resp) if resp.status().is_success() => Ok(()),
        other => {
            let reason = match other {
                Ok(r) => format!("bridge HTTP {}", r.status()),
                Err(e) => e.to_string(),
            };
            queue_fallback(state, &body.to_string()).await?;
            tracing_noop(&reason);
            Ok(())
        }
    }
}

async fn queue_fallback(state: &AppState, payload: &str) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    redis::cmd("XADD")
        .arg(OUTBOUND_STREAM)
        .arg("MAXLEN")
        .arg("~")
        .arg("10000")
        .arg("*")
        .arg("payload")
        .arg(payload)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

fn tracing_noop(_reason: &str) {}
