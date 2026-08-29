mod config;
mod fsm;
mod handlers;
mod outbound;
mod rates;
mod security;
mod state;
mod wallet;
mod worker;

use axum::{
    routing::{get, post},
    Router,
};
use handlers::admin::{bot_interactions, bot_resolve_interaction, bot_stats, create_catalogue, create_employee, dashboard, delete_catalogue, delete_employee, fees_list, fees_update, foreign_accounts_all, get_analytics, get_employee, get_roles, kb_search, list_employees, login, rates_list, rates_refresh, resolve_trade, track_metric, transactions_recent, update_catalogue, update_employee};
use state::AppState;
use std::{net::SocketAddr, sync::Arc};
use tower_http::cors::{Any, CorsLayer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let cfg = config::Config::from_env();
    println!("Starting MiddleMan core API v2.4.0");

    let state = Arc::new(AppState::connect(cfg).await?);

    if state.cfg.auto_migrate {
        sqlx::migrate!("../../migrations").run(&state.pool).await?;
        println!("migrations applied");
    }

    {
        let worker_state = state.clone();
        tokio::spawn(async move {
            worker::run(worker_state).await;
        });
    }
    // auto rates (non-giftcard) every 60s tick, per-pair interval 300s crypto / 3600s fiat, with fallback jitter
    rates::spawn(state.pool.clone());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Auth
        .route("/api/v1/admin/login", post(login))
        // Dashboard
        .route("/api/v1/admin/health", get(handlers::health::health))
        .route("/api/v1/admin/dashboard", get(dashboard))
        // Employee management
        .route("/api/v1/admin/employees", get(list_employees))
        .route("/api/v1/admin/employees", post(create_employee))
        .route("/api/v1/admin/employees/:id", get(get_employee))
        .route("/api/v1/admin/employees/:id", post(update_employee))
        .route("/api/v1/admin/employees/:id", axum::routing::delete(delete_employee))
        // Price catalogue
        .route("/api/v1/admin/catalogue", get(dashboard)) // list via dashboard
        .route("/api/v1/admin/catalogue", post(create_catalogue))
        .route("/api/v1/admin/catalogue/:id", post(update_catalogue))
        .route("/api/v1/admin/catalogue/:id", axum::routing::delete(delete_catalogue))
        // Trades
        .route("/api/v1/admin/trades/:id/resolve", post(resolve_trade))
        // Analytics & Bot (Case Study 1)
        .route("/api/v1/admin/analytics", get(get_analytics))
        .route("/api/v1/admin/analytics/track", post(track_metric))
        .route("/api/v1/admin/bot/stats", get(bot_stats))
        .route("/api/v1/admin/bot/interactions", get(bot_interactions))
        .route("/api/v1/admin/bot/interactions/:id/resolve", post(bot_resolve_interaction))
        .route("/api/v1/admin/kb", get(kb_search))
        .route("/api/v1/admin/transactions", get(transactions_recent))
        .route("/api/v1/admin/foreign-accounts", get(foreign_accounts_all))
        .route("/api/v1/admin/fees", get(fees_list))
        .route("/api/v1/admin/fees/:fee_type", post(fees_update))
        .route("/api/v1/admin/rates", get(rates_list))
        .route("/api/v1/admin/rates/refresh", post(rates_refresh))
        // Roles
        .route("/api/v1/admin/roles", get(get_roles))
        // Debug
        .route("/api/v1/debug/ingest", post(handlers::debug_ingest::ingest))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("listening on {addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
