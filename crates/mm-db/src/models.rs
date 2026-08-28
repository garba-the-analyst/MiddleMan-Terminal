use rust_decimal::Decimal;
use sqlx::types::chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub whatsapp_number: String,
    pub full_name: Option<String>,
    pub current_state: String,
    pub failed_pin_attempts: i32,
    pub pin_locked_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TradeRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub card_brand: String,
    pub country: String,
    pub card_format: String,
    pub claimed_usd_amount: Decimal,
    pub offered_ngn_rate: Decimal,
    pub final_ngn_payout: Decimal,
    pub status: String,
    pub image_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecentTrade {
    #[sqlx(flatten)]
    pub trade: TradeRow,
    pub user_number: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CatalogueRow {
    pub id: i64,
    pub brand: String,
    pub country: String,
    pub card_format: String,
    pub rate_per_dollar: Decimal,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct CreditedTrade {
    pub trade_id: Uuid,
    pub user_id: Uuid,
    pub payout_ngn: Decimal,
    pub brand: String,
    pub claimed_usd: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolveAction {
    Approve,
    Reject,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KeyVaultRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub chain_type: String,
    pub public_address: String,
    pub encrypted_private_key: String,
    pub nonce: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WalletRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub wallet_type: String,
    pub currency: String,
    pub balance: Decimal,
    pub reserved_balance: Decimal,
}
