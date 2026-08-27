use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use mm_db::{queries as db, ResolveAction};
use rust_decimal::Decimal;
use std::sync::Arc;
use uuid::Uuid;

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or_default();
    let a = provided.as_bytes();
    let b = state.cfg.admin_api_token.as_bytes();
    if a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    authorize(&state, &headers)?;

    let users = db::user_count(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let pending = db::pending_count(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let trades = db::recent_trades(&state.pool, 10).await.unwrap_or_default();
    let catalogue = db::catalogue_list(&state.pool).await.unwrap_or_default();

    let trade_json: Vec<serde_json::Value> = trades
        .iter()
        .map(|t| {
            serde_json::json!({
                "db_id": t.trade.id,
                "id": format!("GC-{}", short_id(&t.trade.id)),
                "user": t.user_number.clone().unwrap_or_default(),
                "card": t.trade.card_brand,
                "amount": format!("${}", t.trade.claimed_usd_amount),
                "calculatedNaira": format!("₦{}", t.trade.final_ngn_payout),
                "status": t.trade.status,
                "image_url": t.trade.image_url,
                "time": t.trade.created_at.to_rfc3339(),
            })
        })
        .collect();

    let catalogue_json: Vec<serde_json::Value> = catalogue
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "brand": c.brand,
                "country": c.country,
                "type": c.card_format,
                "ratePerDollar": c.rate_per_dollar,
                "status": if c.active { "Active" } else { "Inactive" },
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "stats": {
            "activeUsers": users,
            "pendingCards": pending,
            "todayVolume": "₦0.00",
        },
        "trades": trade_json,
        "catalogue": catalogue_json,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct ResolveBody {
    pub action: String,
    pub reason: Option<String>,
    pub adjusted_payout: Option<Decimal>,
}

fn short_id(id: &Uuid) -> String {
    id.simple()
        .to_string()
        .chars()
        .take(8)
        .collect::<String>()
        .to_uppercase()
}

pub async fn resolve_trade(
    State(state): State<Arc<AppState>>,
    Path(trade_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ResolveBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers).map_err(|s| (s, Json(serde_json::json!({"error":"unauthorized"}))))?;

    let uuid = Uuid::parse_str(&trade_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid id"}))))?;

    let action = match body.action.as_str() {
        "approve" => ResolveAction::Approve,
        "reject" => ResolveAction::Reject,
        _ => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": "action must be approve|reject"})),
            ))
        }
    };

    let credited = db::resolve_trade(
        &state.pool,
        uuid,
        action,
        None,
        body.reason.as_deref(),
        body.adjusted_payout,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let Some(credited) = credited else {
        return Ok(Json(
            serde_json::json!({ "status": "noop", "detail": "already resolved or missing" }),
        ));
    };

    let state_for_notify = state.clone();
    tokio::spawn(async move {
        let number = db::whatsapp_number_of(&state_for_notify.pool, credited.user_id)
            .await
            .unwrap_or_default();
        if number.is_empty() {
            return;
        }
        let jid = format!("{number}@s.whatsapp.net");
        let text = match action {
            ResolveAction::Approve => format!(
                "*✅ Trade Approved!*\n\nYour {} ${} card has been validated.\n\n*Payout:* ₦{} credited to your NGN wallet.",
                credited.brand, credited.claimed_usd, credited.payout_ngn
            ),
            ResolveAction::Reject => format!(
                "*❌ Trade Rejected*\n\nYour {} card was rejected.\n\nReason: {}",
                credited.brand,
                body.reason
                    .clone()
                    .unwrap_or_else(|| "Card invalid or already redeemed.".into())
            ),
        };
        let _ = crate::outbound::send_text(&state_for_notify, &jid, &text).await;
    });

    println!(
        "trade {} resolved as {:?} by desk",
        short_id(&uuid),
        action
    );

    Ok(Json(serde_json::json!({
        "status": match action { ResolveAction::Approve => "approved", ResolveAction::Reject => "rejected" },
        "trade_id": uuid,
    })))
}
