use crate::config::Config;
use mm_ai::parser::GeminiParser;
use mm_vault::VaultAead;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};

pub struct AppState {
    pub cfg: Config,
    pub pool: Pool<Postgres>,
    pub redis: redis::Client,
    pub http: reqwest::Client,
    pub ai: GeminiParser,
    pub vault: Option<VaultAead>,
}

impl AppState {
    pub async fn get_user_pin_hash(state: &AppState, user_id: uuid::Uuid) -> Option<String> {
        mm_db::queries::get_user_pin_hash(&state.pool, user_id).await.ok().flatten()
    }
    pub async fn reset_pin_attempts(state: &AppState, user_id: uuid::Uuid) {
        let _ = mm_db::queries::reset_pin_attempts(&state.pool, user_id).await;
    }
    pub async fn connect(cfg: Config) -> anyhow::Result<Self> {
        let redis_client = redis::Client::open(cfg.redis_url.clone())?;
        let ai = GeminiParser::new(cfg.gemini_api_key.clone().unwrap_or_default());
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&cfg.database_url)
            .await?;

        let vault = std::env::var("MM_MASTER_KEY")
            .ok()
            .and_then(|k| VaultAead::from_hex_master(&k).ok());

        if vault.is_none() {
            eprintln!("warning: MM_MASTER_KEY not set or invalid — wallet encryption disabled");
        }

        Ok(Self {
            cfg,
            pool,
            redis: redis_client,
            http: reqwest::Client::new(),
            ai,
            vault,
        })
    }
}
