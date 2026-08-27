mod config;
mod fsm;
mod handlers;
mod outbound;
mod state;
mod worker;

use axum::{
    routing::{get, post},
    Router,
};
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

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/v1/admin/health", get(handlers::health::health))
        .route("/api/v1/admin/dashboard", get(handlers::admin::dashboard))
        .route("/api/v1/admin/trades/:id/resolve", post(handlers::admin::resolve_trade))
        .route("/api/v1/debug/ingest", post(handlers::debug_ingest::ingest))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("listening on {addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
