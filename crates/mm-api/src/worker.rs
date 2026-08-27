use crate::fsm::{advance, FsmInput};
use crate::state::AppState;
use serde::Deserialize;
use std::sync::Arc;

const STREAM: &str = "inbound:wa:events";
const DLQ: &str = "inbound:wa:dlq";
const GROUP: &str = "mm_api_workers";
const CONSUMER: &str = "worker-1";

#[derive(Debug, Deserialize)]
struct InboundEvent {
    message_id: String,
    sender_jid: String,
    #[serde(default)]
    text_body: String,
    #[serde(default)]
    media_url: Option<String>,
}

fn as_array(v: &redis::Value) -> Option<&Vec<redis::Value>> {
    match v {
        redis::Value::Array(items) => Some(items),
        _ => None,
    }
}

fn as_string(v: &redis::Value) -> Option<String> {
    match v {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

fn extract_entries(raw: redis::Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(outer) = as_array(&raw) else { return out };
    for stream in outer {
        let Some(stream_pair) = as_array(stream) else { continue };
        if stream_pair.len() != 2 {
            continue;
        }
        let Some(entries) = as_array(&stream_pair[1]) else { continue };
        for entry in entries {
            let Some(pair) = as_array(entry) else { continue };
            if pair.len() != 2 {
                continue;
            }
            let Some(id) = as_string(&pair[0]) else { continue };
            let payload = as_array(&pair[1])
                .and_then(|fields| {
                    fields.chunks(2).find_map(|chunk| {
                        match (as_string(&chunk[0]), chunk.get(1).and_then(as_string)) {
                            (Some(k), v) if k == "payload" => v,
                            _ => None,
                        }
                    })
                })
                .unwrap_or_default();
            out.push((id, payload));
        }
    }
    out
}

pub async fn run(state: Arc<AppState>) {
    let mut conn = loop {
        match state.redis.get_multiplexed_async_connection().await {
            Ok(c) => break c,
            Err(e) => {
                eprintln!("redis connect failed: {e}; retrying in 2s");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    };

    let _: Result<redis::Value, redis::RedisError> = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(STREAM)
        .arg(GROUP)
        .arg("0")
        .arg("MKSTREAM")
        .query_async(&mut conn)
        .await;

    println!("worker consuming group={GROUP} on {STREAM}");

    loop {
        let raw = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(GROUP)
            .arg(CONSUMER)
            .arg("COUNT")
            .arg("8")
            .arg("BLOCK")
            .arg("1000")
            .arg("STREAMS")
            .arg(STREAM)
            .arg(">")
            .query_async::<redis::Value>(&mut conn)
            .await;

        let raw = match raw {
            Ok(v) => v,
            Err(e) => {
                eprintln!("xreadgroup error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        for (entry_id, payload) in extract_entries(raw) {
            process(&state, &mut conn, &entry_id, &payload).await;
        }
    }
}

async fn ack_and_delete(
    conn: &mut impl redis::aio::ConnectionLike,
    entry_id: &str,
) -> Result<(), redis::RedisError> {
    redis::cmd("XACK")
        .arg(STREAM)
        .arg(GROUP)
        .arg(entry_id)
        .query_async::<()>(&mut *conn)
        .await?;
    redis::cmd("XDEL")
        .arg(STREAM)
        .arg(entry_id)
        .query_async::<()>(&mut *conn)
        .await?;
    Ok(())
}

async fn dead_letter(
    conn: &mut impl redis::aio::ConnectionLike,
    entry_id: &str,
    payload: &str,
    class: &str,
    detail: &str,
) {
    let envelope = serde_json::json!({
        "source_entry": entry_id,
        "error_class": class,
        "last_error": detail,
        "at": sqlx::types::chrono::Utc::now().timestamp(),
        "original": serde_json::from_str::<serde_json::Value>(payload)
            .unwrap_or(serde_json::Value::String(payload.to_string())),
    });
    let _: Result<(), _> = redis::cmd("XADD")
        .arg(DLQ)
        .arg("*")
        .arg("payload")
        .arg(envelope.to_string())
        .query_async(&mut *conn)
        .await;
}

enum Outcome {
    Done,
    Retry(String),
    Terminal(String),
}

fn sender_number(jid: &str) -> Option<String> {
    jid.split('@')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

async fn process(state: &AppState, conn: &mut impl redis::aio::ConnectionLike, entry_id: &str, payload: &str) {
    let outcome = run_once(state, payload).await;

    let result = match outcome {
        Outcome::Done => {
            if let Err(e) = ack_and_delete(conn, entry_id).await {
                eprintln!("ack failed for {entry_id}: {e}");
            }
            return;
        }
        Outcome::Retry(d) => Outcome::Retry(d),
        Outcome::Terminal(d) => Outcome::Terminal(d),
    };

    match result {
        Outcome::Retry(detail) => {
            eprintln!("retryable failure on {entry_id}: {detail}");
            dead_letter(conn, entry_id, payload, "RETRY", &detail).await;
            let _ = ack_and_delete(conn, entry_id).await;
        }
        Outcome::Terminal(detail) => {
            eprintln!("terminal failure on {entry_id}: {detail}");
            dead_letter(conn, entry_id, payload, "TERMINAL", &detail).await;
            let _ = ack_and_delete(conn, entry_id).await;
        }
        Outcome::Done => unreachable!(),
    }
}

async fn run_once(state: &AppState, payload: &str) -> Outcome {
    let event: InboundEvent = match serde_json::from_str(payload) {
        Ok(e) => e,
        Err(e) => return Outcome::Terminal(format!("malformed payload: {e}")),
    };

    match db_claim(state, &event.message_id).await {
        Ok(false) => return Outcome::Done,
        Ok(true) => {}
        Err(e) => return Outcome::Retry(e),
    }

    let Some(number) = sender_number(&event.sender_jid) else {
        return Outcome::Terminal("sender_jid missing user part".into());
    };

    match advance(
        state,
        FsmInput {
            message_id: event.message_id.clone(),
            whatsapp_number: number,
            text: event.text_body,
            media_url: event.media_url,
        },
    )
    .await
    {
        Ok(()) => Outcome::Done,
        Err(e) if e.is_retryable() => Outcome::Retry(e.to_string()),
        Err(e) => Outcome::Terminal(e.to_string()),
    }
}

async fn db_claim(state: &AppState, message_id: &str) -> Result<bool, String> {
    mm_db::queries::claim_message(&state.pool, message_id)
        .await
        .map_err(|e| e.to_string())
}
