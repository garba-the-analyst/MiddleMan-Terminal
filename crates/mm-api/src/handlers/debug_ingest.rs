use crate::state::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct IngestBody {
    pub message_id: String,
    pub sender_jid: String,
    #[serde(default)]
    pub text_body: String,
    #[serde(default)]
    pub media_url: Option<String>,
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let provided = headers
        .get("X-Internal-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let a = provided.as_bytes();
    let b = state.cfg.internal_api_secret.as_bytes();
    if a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Test harness endpoint: publishes a bridge-shaped event onto the inbound stream.
pub async fn ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)
        .map_err(|s| (s, Json(serde_json::json!({"error": "unauthorized"}))))?;

    let body: IngestBody = serde_json::from_slice(&bytes).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let payload = serde_json::json!({
        "message_id": body.message_id,
        "sender_jid": body.sender_jid,
        "text_body": body.text_body,
        "has_media": body.media_url.is_some(),
        "media_url": body.media_url,
        "timestamp": sqlx::types::chrono::Utc::now().timestamp(),
    });

    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    let entry: String = redis::cmd("XADD")
        .arg("inbound:wa:events")
        .arg("MAXLEN")
        .arg("~")
        .arg("10000")
        .arg("*")
        .arg("payload")
        .arg(payload.to_string())
        .query_async(&mut conn)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    Ok(Json(serde_json::json!({ "queued": entry })))
}
