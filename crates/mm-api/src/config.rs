use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub internal_api_secret: String,
    pub admin_api_token: String,
    pub wa_bridge_url: String,
    pub gemini_api_key: Option<String>,
    pub auto_migrate: bool,
}

fn required(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("{key} must be set"))
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: required("DATABASE_URL"),
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into()),
            internal_api_secret: required("INTERNAL_API_SECRET"),
            admin_api_token: required("ADMIN_API_TOKEN"),
            wa_bridge_url: env::var("WA_BRIDGE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:3001".into()),
            gemini_api_key: env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty()),
            auto_migrate: env::var("MM_AUTO_MIGRATE")
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(false),
        }
    }
}
