use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use mm_db::{queries as db, ResolveAction};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

const ADMIN_TOKEN_HEADER: &str = "x-admin-token";

fn get_admin_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(ADMIN_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn authorize(state: &AppState, token: &str) -> Result<(), StatusCode> {
    let a = token.as_bytes();
    let b = state.cfg.admin_api_token.as_bytes();
    if a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn authorize_employee(state: &AppState, headers: &HeaderMap) -> Result<Uuid, StatusCode> {
    let token = get_admin_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let pool = &state.pool;
    let emp_id = db::validate_admin_token(&pool, &token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(emp_id)
}

async fn check_permission(state: &AppState, emp_id: Uuid, permission: &str) -> Result<(), StatusCode> {
    let pool = &state.pool;
    let has_perm = db::has_permission(pool, emp_id, permission)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if has_perm {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let emp_id = authorize_employee(&state, &headers).await?;
    check_permission(&state, emp_id, "trades.read").await?;

    let users = db::user_count(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let pending = db::pending_count(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let trades = db::recent_trades(&state.pool, 20).await.unwrap_or_default();
    let catalogue = db::catalogue_list(&state.pool).await.unwrap_or_default();

    let trade_json: Vec<serde_json::Value> = trades
        .iter()
        .map(|t| {
            let display_status = match t.trade.status.as_str() {
                "PENDING" => "Pending Review",
                "APPROVED" => "Approved",
                "REJECTED" => "Rejected",
                other => other,
            };
            serde_json::json!({
                "db_id": t.trade.id,
                "id": format!("GC-{}", short_id(&t.trade.id)),
                "user": t.user_number.clone().unwrap_or_default(),
                "card": t.trade.card_brand,
                "amount": format!("${}", t.trade.claimed_usd_amount),
                "calculatedNaira": format!("₦{}", t.trade.final_ngn_payout),
                "status": display_status,
                "raw_status": t.trade.status,
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

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub employee: EmployeeResponse,
}

#[derive(Debug, Serialize)]
pub struct EmployeeResponse {
    pub id: Uuid,
    pub email: String,
    pub full_name: Option<String>,
    pub role: String,
    pub permissions: serde_json::Value,
    pub is_active: bool,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<serde_json::Value>)> {
    let pool = &state.pool;
    let emp = db::get_admin_by_email(pool, &body.email)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?
        .ok_or((StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid credentials"}))))?;

    if !emp.is_active {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "account deactivated"}))));
    }

    let valid = mm_vault::pin_hash::verify_pin(&body.password, &emp.password_hash)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    if !valid {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid credentials"}))));
    }

    let token = db::create_admin_token(pool, emp.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    db::update_admin_last_login(pool, emp.id).await.ok();

    Ok(Json(LoginResponse {
        token,
        employee: EmployeeResponse {
            id: emp.id,
            email: emp.email,
            full_name: emp.full_name,
            role: emp.role,
            permissions: emp.permissions,
            is_active: emp.is_active,
        },
    }))
}

#[derive(Debug, Deserialize)]
pub struct CreateEmployeeBody {
    pub email: String,
    pub password: String,
    pub full_name: Option<String>,
    pub role: String,
    pub permissions: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEmployeeBody {
    pub full_name: Option<String>,
    pub role: Option<String>,
    pub permissions: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

pub async fn create_employee(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateEmployeeBody>,
) -> Result<Json<EmployeeResponse>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err((e, Json(serde_json::json!({"error": "unauthorized"})))),
    };
    if let Err(e) = check_permission(&state, emp_id, "employees.create").await {
        return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
    }

    let pool = &state.pool;
    let password_hash = mm_vault::pin_hash::hash_password(&body.password)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    let emp = db::create_admin_employee(
        pool,
        &body.email,
        &password_hash,
        body.full_name.as_deref(),
        &body.role,
        body.permissions.unwrap_or(serde_json::json!([])),
        Some(emp_id),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    Ok(Json(EmployeeResponse {
        id: emp.id,
        email: emp.email,
        full_name: emp.full_name,
        role: emp.role,
        permissions: emp.permissions,
        is_active: emp.is_active,
    }))
}

pub async fn list_employees(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<EmployeeResponse>>, StatusCode> {
    let emp_id = authorize_employee(&state, &headers).await?;
    check_permission(&state, emp_id, "employees.read").await?;

    let pool = &state.pool;
    let employees = db::list_admin_employees(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        employees
            .into_iter()
            .map(|e| EmployeeResponse {
                id: e.id,
                email: e.email,
                full_name: e.full_name,
                role: e.role,
                permissions: e.permissions,
                is_active: e.is_active,
            })
            .collect(),
    ))
}

pub async fn get_employee(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<EmployeeResponse>, StatusCode> {
    let emp_id = authorize_employee(&state, &headers).await?;
    check_permission(&state, emp_id, "employees.read").await?;

    let pool = &state.pool;
    let emp = db::get_admin_employee(pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(EmployeeResponse {
        id: emp.id,
        email: emp.email,
        full_name: emp.full_name,
        role: emp.role,
        permissions: emp.permissions,
        is_active: emp.is_active,
    }))
}

pub async fn update_employee(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateEmployeeBody>,
) -> Result<Json<EmployeeResponse>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err((e, Json(serde_json::json!({"error": "unauthorized"})))),
    };
    if let Err(e) = check_permission(&state, emp_id, "employees.update").await {
        return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
    }

    let pool = &state.pool;
    let emp = db::update_admin_employee(
        pool,
        id,
        body.full_name,
        body.role,
        body.permissions,
        body.is_active,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?
    .ok_or((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "employee not found"}))))?;

    Ok(Json(EmployeeResponse {
        id: emp.id,
        email: emp.email,
        full_name: emp.full_name,
        role: emp.role,
        permissions: emp.permissions,
        is_active: emp.is_active,
    }))
}

pub async fn delete_employee(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err((e, Json(serde_json::json!({"error": "unauthorized"})))),
    };
    if let Err(e) = check_permission(&state, emp_id, "employees.delete").await {
        return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
    }

    if emp_id == id {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "cannot delete yourself"}))));
    }

    let pool = &state.pool;
    db::delete_admin_employee(pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    Ok(Json(serde_json::json!({"status": "deleted"})))
}

#[derive(Debug, Deserialize)]
pub struct CatalogueBody {
    pub brand: String,
    pub country: String,
    pub card_format: String,
    pub rate_per_dollar: Decimal,
    pub active: Option<bool>,
}

pub async fn create_catalogue(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CatalogueBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err((e, Json(serde_json::json!({"error": "unauthorized"})))),
    };
    if let Err(e) = check_permission(&state, emp_id, "catalogue.create").await {
        return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
    }

    let pool = &state.pool;
    let cat = db::create_price_catalogue(
        pool,
        &body.brand,
        &body.country,
        &body.card_format,
        body.rate_per_dollar,
        body.active.unwrap_or(true),
        Some(emp_id),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    Ok(Json(serde_json::json!({
        "id": cat.id,
        "brand": cat.brand,
        "country": cat.country,
        "type": cat.card_format,
        "ratePerDollar": cat.rate_per_dollar,
        "status": if cat.active { "Active" } else { "Inactive" },
    })))
}

pub async fn update_catalogue(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<CatalogueBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err((e, Json(serde_json::json!({"error": "unauthorized"})))),
    };
    if let Err(e) = check_permission(&state, emp_id, "catalogue.update").await {
        return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
    }

    let pool = &state.pool;
    let cat = db::update_price_catalogue(
        pool,
        id,
        Some(&body.brand),
        Some(&body.country),
        Some(&body.card_format),
        Some(body.rate_per_dollar),
        body.active,
        Some(emp_id),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?
    .ok_or((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "catalogue not found"}))))?;

    Ok(Json(serde_json::json!({
        "id": cat.id,
        "brand": cat.brand,
        "country": cat.country,
        "type": cat.card_format,
        "ratePerDollar": cat.rate_per_dollar,
        "status": if cat.active { "Active" } else { "Inactive" },
    })))
}

pub async fn delete_catalogue(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err((e, Json(serde_json::json!({"error": "unauthorized"})))),
    };
    if let Err(e) = check_permission(&state, emp_id, "catalogue.delete").await {
        return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
    }

    let pool = &state.pool;
    db::delete_price_catalogue(pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    Ok(Json(serde_json::json!({"status": "deleted"})))
}

#[derive(Debug, Deserialize)]
pub struct AnalyticsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub metric: Option<String>,
}

pub async fn get_analytics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AnalyticsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let emp_id = authorize_employee(&state, &headers).await?;
    check_permission(&state, emp_id, "analytics.read").await?;

    let pool = &state.pool;
    let analytics = db::get_bot_analytics(
        pool,
        query.from.as_deref(),
        query.to.as_deref(),
        query.metric.as_deref(),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let analytics_json: Vec<serde_json::Value> = analytics
        .into_iter()
        .map(|a| serde_json::json!({
            "id": a.id,
            "date": a.date.to_string(),
            "metric_name": a.metric_name,
            "metric_value": a.metric_value,
            "metadata": a.metadata,
        }))
        .collect();

    Ok(Json(analytics_json))
}

#[derive(Debug, Deserialize)]
pub struct TrackMetricBody {
    pub metric_name: String,
    pub metric_value: i64,
    pub metadata: Option<serde_json::Value>,
}

pub async fn track_metric(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TrackMetricBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let pool = &state.pool;
    db::upsert_bot_analytics(pool, &body.metric_name, body.metric_value, body.metadata)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

    Ok(Json(serde_json::json!({"status": "recorded"})))
}

pub async fn get_roles(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err(e),
    };
    if let Err(e) = check_permission(&state, emp_id, "employees.read").await {
        return Err(e);
    }

    let pool = &state.pool;
    let roles = db::get_role_permissions(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(roles))
}

fn short_id(id: &Uuid) -> String {
    id.simple()
        .to_string()
        .chars()
        .take(8)
        .collect::<String>()
        .to_uppercase()
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    pub action: Option<String>,
    pub status: Option<String>,
    pub reason: Option<String>,
    pub adjusted_payout: Option<Decimal>,
}

impl ResolveBody {
    fn effective_action(&self) -> Option<String> {
        self.action
            .clone()
            .or_else(|| self.status.clone())
            .map(|s| s.to_lowercase())
    }
}

pub async fn resolve_trade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(trade_id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await {
        Ok(id) => id,
        Err(e) => return Err((e, Json(serde_json::json!({"error": "unauthorized"})))),
    };
    
    let action_str = body.effective_action().unwrap_or_else(|| "unknown".into());
    let action = match action_str.as_str() {
        "approve" | "approved" => {
            if let Err(e) = check_permission(&state, emp_id, "trades.approve").await {
                return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
            }
            ResolveAction::Approve
        }
        "reject" | "rejected" => {
            if let Err(e) = check_permission(&state, emp_id, "trades.reject").await {
                return Err((e, Json(serde_json::json!({"error": "forbidden"}))));
            }
            ResolveAction::Reject
        }
        _ => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": "action must be approve|reject"})),
            ))
        }
    };

    let uuid = Uuid::parse_str(&trade_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid id"}))))?;

    let credited = db::resolve_trade(
        &state.pool,
        uuid,
        action,
        Some(emp_id),
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
        "trade {} resolved as {:?} by employee {}",
        short_id(&uuid),
        action,
        emp_id
    );

    Ok(Json(serde_json::json!({
        "status": match action { ResolveAction::Approve => "approved", ResolveAction::Reject => "rejected" },
        "trade_id": uuid,
    })))
}

pub async fn health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({
        "status": "ok",
        "db": !state.pool.is_closed(),
        "redis": true,
        "ai_enabled": true,
    })))
}

// ========== Bot Case Study 1 endpoints ==========

pub async fn bot_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let emp_id = authorize_employee(&state, &headers).await?;
    check_permission(&state, emp_id, "analytics.read").await?;
    let stats = db::get_bot_stats(&state.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({
        "total_interactions": stats.total_interactions,
        "today_interactions": stats.today_interactions,
        "escalated_count": stats.escalated_count,
        "auto_resolved": stats.auto_resolved,
        "escalation_rate": if stats.total_interactions>0 { (stats.escalated_count as f64 / stats.total_interactions as f64 * 100.0) } else { 0.0 },
        "avg_handling_ms": stats.avg_handling_ms,
        "by_category": stats.by_category.into_iter().map(|(k,v)| serde_json::json!({"name":k,"value":v})).collect::<Vec<_>>(),
        "by_sentiment": stats.by_sentiment.into_iter().map(|(k,v)| serde_json::json!({"name":k,"value":v})).collect::<Vec<_>>(),
        "by_urgency": stats.by_urgency.into_iter().map(|(k,v)| serde_json::json!({"name":k,"value":v})).collect::<Vec<_>>(),
        "by_intent": stats.by_intent.into_iter().map(|(k,v)| serde_json::json!({"name":k,"value":v})).collect::<Vec<_>>(),
        "last_14_days": stats.last_14_days.into_iter().map(|(d,v)| serde_json::json!({"date":d,"value":v})).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct BotInteractionsQuery {
    pub escalated_only: Option<bool>,
    pub limit: Option<i64>,
}

pub async fn bot_interactions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<BotInteractionsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let emp_id = authorize_employee(&state, &headers).await?;
    check_permission(&state, emp_id, "analytics.read").await?;
    let rows = db::list_bot_interactions(&state.pool, q.limit.unwrap_or(50).clamp(1,200), q.escalated_only.unwrap_or(false)).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let json: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({
        "id": r.id,
        "whatsapp_number": r.whatsapp_number,
        "inbound_text": r.inbound_text,
        "intent": r.intent,
        "category": r.category,
        "sentiment": r.sentiment,
        "urgency": r.urgency,
        "urgency_score": r.urgency_score,
        "confidence": r.confidence,
        "response_text": r.response_text,
        "escalated": r.escalated,
        "escalation_reason": r.escalation_reason,
        "resolved": r.resolved,
        "handling_ms": r.handling_ms,
        "created_at": r.created_at.to_rfc3339(),
    })).collect();
    Ok(Json(json))
}

pub async fn bot_resolve_interaction(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let emp_id = match authorize_employee(&state, &headers).await { Ok(id)=>id, Err(e)=> return Err((e, Json(serde_json::json!({"error":"unauthorized"})))) };
    if let Err(e) = check_permission(&state, emp_id, "analytics.read").await { return Err((e, Json(serde_json::json!({"error":"forbidden"})))) }
    db::resolve_bot_interaction(&state.pool, id, emp_id).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":e.to_string()}))))?;
    Ok(Json(serde_json::json!({"status":"resolved"})))
}

pub async fn kb_search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String,String>>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let pool = &state.pool;
    let query = q.get("q").map(|s| s.as_str()).unwrap_or("");
    let cat = q.get("category").map(|s| s.as_str());
    let rows = if query.is_empty() {
        db::knowledge_base_list(pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        db::knowledge_base_search(pool, query, cat, 20).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    let json: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({"id":r.id,"category":r.category,"question":r.question,"answer":r.answer,"priority":r.priority})).collect();
    Ok(Json(json))
}