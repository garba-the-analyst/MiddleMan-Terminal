use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

pub async fn health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();

    let redis_ok = match state.redis.get_multiplexed_async_connection().await {
        Ok(mut conn) => redis::cmd("PING")
            .query_async::<()>(&mut conn)
            .await
            .is_ok(),
        Err(_) => false,
    };

    Ok(Json(serde_json::json!({
        "status": if db_ok && redis_ok { "ok" } else { "degraded" },
        "db": db_ok,
        "redis": redis_ok,
        "ai_enabled": state.ai.is_enabled(),
    })))
}
