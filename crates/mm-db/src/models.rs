use rust_decimal::Decimal;
use serde::Serialize;
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
pub struct AdminEmployee {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub full_name: Option<String>,
    pub role: String,
    pub permissions: serde_json::Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct PriceCatalogue {
    pub id: i64,
    pub brand: String,
    pub country: String,
    pub card_format: String,
    pub rate_per_dollar: Decimal,
    pub active: bool,
    pub created_by: Option<Uuid>,
    pub updated_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BotAnalyticsRow {
    pub id: Uuid,
    pub date: chrono::NaiveDate,
    pub metric_name: String,
    pub metric_value: i64,
    pub metadata: serde_json::Value,
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

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BotInteraction {
    pub id: Uuid,
    pub message_id: Option<String>,
    pub whatsapp_number: String,
    pub user_id: Option<Uuid>,
    pub inbound_text: String,
    pub intent: String,
    pub category: String,
    pub sentiment: String,
    pub urgency: String,
    pub urgency_score: i32,
    pub confidence: Option<Decimal>,
    pub response_text: Option<String>,
    pub escalated: bool,
    pub escalation_reason: Option<String>,
    pub assigned_agent: Option<Uuid>,
    pub resolved: bool,
    pub handling_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KnowledgeBaseRow {
    pub id: Uuid,
    pub category: String,
    pub question: String,
    pub answer: String,
    pub keywords: Option<Vec<String>>,
    pub source: Option<String>,
    pub priority: Option<i32>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct BotStats {
    pub total_interactions: i64,
    pub today_interactions: i64,
    pub escalated_count: i64,
    pub auto_resolved: i64,
    pub avg_handling_ms: i64,
    pub by_category: Vec<(String, i64)>,
    pub by_sentiment: Vec<(String, i64)>,
    pub by_urgency: Vec<(String, i64)>,
    pub by_intent: Vec<(String, i64)>,
    pub last_14_days: Vec<(String, i64)>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FiatPayoutRow { pub id: Uuid, pub user_id: Uuid, pub amount: Decimal, pub currency: String, pub bank_code: String, pub account_number: String, pub account_name: Option<String>, pub provider_ref: Option<String>, pub status: String, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CryptoTransferRow { pub id: Uuid, pub user_id: Uuid, pub chain_type: String, pub token: String, pub amount: Decimal, pub recipient_address: String, pub tx_hash: Option<String>, pub status: String, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AirtimeRow { pub id: Uuid, pub user_id: Uuid, pub recipient_phone: String, pub network: String, pub amount: Decimal, pub purchase_type: String, pub status: String, pub provider_ref: Option<String>, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ForeignAccountRow { pub id: Uuid, pub user_id: Uuid, pub currency: String, pub account_number: String, pub provider: String, pub status: String, pub created_at: DateTime<Utc> }
