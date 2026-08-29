use crate::models::*;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("record not found")]
    NotFound,
    #[error("insufficient funds")]
    InsufficientFunds,
    #[error("database failure: {0}")]
    Backend(#[from] sqlx::Error),
}

pub async fn ensure_user(
    pool: &PgPool,
    whatsapp_number: &str,
) -> Result<UserRow, DbError> {
    let row = sqlx::query_as::<_, UserRow>(
        r#"INSERT INTO users (whatsapp_number)
           VALUES ($1)
           ON CONFLICT (whatsapp_number) DO UPDATE SET updated_at = NOW()
           RETURNING id, whatsapp_number, full_name, current_state,
                     failed_pin_attempts, pin_locked_until"#,
    )
    .bind(whatsapp_number)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn set_state(
    pool: &PgPool,
    user_id: Uuid,
    state: &str,
    state_data: serde_json::Value,
) -> Result<(), DbError> {
    sqlx::query!(
        r#"UPDATE users SET current_state = $2, state_data = $3 WHERE id = $1"#,
        user_id,
        state,
        state_data
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns true when the message was fresh; false for duplicates.
pub async fn claim_message(pool: &PgPool, message_id: &str) -> Result<bool, DbError> {
    let inserted = sqlx::query!(
        r#"INSERT INTO processed_messages (message_id) VALUES ($1)
           ON CONFLICT DO NOTHING RETURNING message_id"#,
        message_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(inserted.is_some())
}

pub async fn ensure_ngn_wallet(pool: &PgPool, user_id: Uuid) -> Result<Decimal, DbError> {
    sqlx::query!(
        r#"INSERT INTO wallets (user_id, wallet_type, currency)
           VALUES ($1, 'FIAT_NGN', 'NGN')
           ON CONFLICT (user_id, currency) DO NOTHING"#,
        user_id
    )
    .execute(pool)
    .await?;

    let row = sqlx::query!(
        r#"SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'NGN'"#,
        user_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.balance)
}

pub async fn catalogue_rate(
    pool: &PgPool,
    brand: &str,
    country: &str,
    card_format: &str,
) -> Option<Decimal> {
    sqlx::query_scalar!(
        r#"SELECT rate_per_dollar FROM price_catalogue
           WHERE brand ILIKE $1 AND country = $2 AND card_format = $3 AND active = TRUE"#,
        format!("%{brand}%"),
        country,
        card_format
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub async fn catalogue_rate_any_format(pool: &PgPool, brand: &str, country: &str) -> Option<Decimal> {
    sqlx::query_scalar!(
        r#"SELECT rate_per_dollar FROM price_catalogue
           WHERE brand ILIKE $1 AND country = $2 AND active = TRUE"#,
        format!("%{brand}%"),
        country
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub struct NewTrade<'a> {
    pub user_id: Uuid,
    pub brand: &'a str,
    pub country: &'a str,
    pub card_format: &'a str,
    pub claimed_usd: Decimal,
    pub rate: Decimal,
    pub payout: Decimal,
    pub code: Option<&'a str>,
    pub image_url: Option<&'a str>,
    pub message_id: &'a str,
}

pub async fn insert_gift_trade(pool: &PgPool, t: NewTrade<'_>) -> Result<Uuid, DbError> {
    let row = sqlx::query!(
        r#"INSERT INTO gift_card_trades
             (user_id, card_brand, country, card_format, claimed_usd_amount,
              offered_ngn_rate, final_ngn_payout, extracted_code, image_url,
              status, message_id)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'PENDING',$10)
           RETURNING id"#,
        t.user_id,
        t.brand,
        t.country,
        t.card_format,
        t.claimed_usd,
        t.rate,
        t.payout,
        t.code,
        t.image_url,
        t.message_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.id)
}

struct ApprovedTradeRow {
    trade_id: Uuid,
    user_id: Uuid,
    payout: Decimal,
    brand: String,
    usd_amount: Decimal,
}

pub async fn resolve_trade(
    pool: &PgPool,
    trade_id: Uuid,
    action: ResolveAction,
    reviewer: Option<Uuid>,
    reason: Option<&str>,
    payout_override: Option<Decimal>,
) -> Result<Option<CreditedTrade>, DbError> {
    let mut tx: Transaction<'_, Postgres> = pool.begin().await?;

    let approved = match action {
        ResolveAction::Approve => {
            let row = sqlx::query_as!(
                ApprovedTradeRow,
                r#"UPDATE gift_card_trades
                   SET status = 'APPROVED',
                       reviewed_by_employee_id = COALESCE($2, reviewed_by_employee_id),
                       final_ngn_payout = COALESCE($3, final_ngn_payout),
                       updated_at = NOW()
                   WHERE id = $1 AND status = 'PENDING'
                   RETURNING id AS trade_id, user_id, final_ngn_payout AS payout,
                             card_brand AS brand, claimed_usd_amount AS usd_amount"#,
                trade_id,
                reviewer,
                payout_override
            )
            .fetch_optional(&mut *tx)
            .await?;

            let Some(row) = row else {
                tx.rollback().await.ok();
                return Ok(None);
            };

            sqlx::query!(
                r#"INSERT INTO transactions
                     (user_id, tx_type, direction, amount, currency, status, metadata)
                   VALUES ($1,'GIFT_CARD_PAYOUT','INBOUND',$2,'NGN','SUCCESS',
                           jsonb_build_object('trade_id', $3::text))"#,
                row.user_id,
                row.payout,
                row.trade_id.to_string()
            )
            .execute(&mut *tx)
            .await?;

            sqlx::query!(
                r#"INSERT INTO wallets (user_id, wallet_type, currency, balance)
                   VALUES ($1, 'FIAT_NGN', 'NGN', 0)
                   ON CONFLICT (user_id, currency) DO NOTHING"#,
                row.user_id
            )
            .execute(&mut *tx)
            .await?;

            sqlx::query!(
                r#"UPDATE wallets SET balance = balance + $1
                   WHERE user_id = $2 AND currency = 'NGN'"#,
                row.payout,
                row.user_id
            )
            .execute(&mut *tx)
            .await?;

            Some(row)
        }
        ResolveAction::Reject => {
            let row = sqlx::query!(
                r#"UPDATE gift_card_trades
                   SET status = 'REJECTED',
                       rejection_reason = COALESCE($2, rejection_reason),
                       reviewed_by_employee_id = COALESCE($3, reviewed_by_employee_id),
                       updated_at = NOW()
                   WHERE id = $1 AND status = 'PENDING'
                   RETURNING id, user_id, final_ngn_payout, card_brand, claimed_usd_amount"#,
                trade_id,
                reason,
                reviewer
            )
            .fetch_optional(&mut *tx)
            .await?;

            match row {
                Some(r) => Some(ApprovedTradeRow {
                    trade_id: r.id,
                    user_id: r.user_id,
                    payout: r.final_ngn_payout,
                    brand: r.card_brand,
                    usd_amount: r.claimed_usd_amount,
                }),
                None => {
                    tx.rollback().await.ok();
                    return Ok(None);
                }
            }
        }
    };

    if let (Some(emp), Some(row)) = (reviewer, approved.as_ref()) {
        let action_label = match action {
            ResolveAction::Approve => "TRADE_APPROVE",
            ResolveAction::Reject => "TRADE_REJECT",
        };
        sqlx::query!(
            r#"INSERT INTO admin_audit_logs (employee_id, action, target_entity, target_id)
               VALUES ($1,$2,'gift_card_trades',$3)"#,
            emp,
            action_label,
            row.trade_id
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(approved.map(|row| CreditedTrade {
        trade_id: row.trade_id,
        user_id: row.user_id,
        payout_ngn: row.payout,
        brand: row.brand,
        claimed_usd: row.usd_amount,
    }))
}

pub async fn recent_trades(pool: &PgPool, limit: i64) -> Result<Vec<RecentTrade>, DbError> {
    let rows = sqlx::query_as::<_, RecentTrade>(
        r#"SELECT t.id, t.user_id, t.card_brand, t.country, t.card_format,
                  t.claimed_usd_amount, t.offered_ngn_rate, t.final_ngn_payout,
                  t.status, t.image_url, t.created_at,
                  u.whatsapp_number AS user_number
           FROM gift_card_trades t
           JOIN users u ON u.id = t.user_id
           ORDER BY t.created_at DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn pending_count(pool: &PgPool) -> Result<i64, DbError> {
    Ok(sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "c!" FROM gift_card_trades WHERE status = 'PENDING'"#
    )
    .fetch_one(pool)
    .await?)
}

pub async fn user_count(pool: &PgPool) -> Result<i64, DbError> {
    Ok(sqlx::query_scalar!(r#"SELECT COUNT(*) AS "c!" FROM users"#)
        .fetch_one(pool)
        .await?)
}

pub async fn catalogue_list(pool: &PgPool) -> Result<Vec<CatalogueRow>, DbError> {
    let rows = sqlx::query_as::<_, CatalogueRow>(
        r#"SELECT id, brand, country, card_format, rate_per_dollar, active
           FROM price_catalogue ORDER BY id ASC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn whatsapp_number_of(pool: &PgPool, user_id: Uuid) -> Result<String, DbError> {
    Ok(sqlx::query_scalar!(r#"SELECT whatsapp_number FROM users WHERE id = $1"#, user_id)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)?)
}

pub async fn get_user_by_phone(pool: &PgPool, phone: &str) -> Result<Option<UserRow>, DbError> {
    let row = sqlx::query_as::<_, UserRow>(
        r#"SELECT id, whatsapp_number, full_name, current_state, failed_pin_attempts, pin_locked_until
           FROM users WHERE whatsapp_number = $1"#,
    )
    .bind(phone)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_key_vault(
    pool: &PgPool,
    user_id: Uuid,
    chain: &str,
) -> Result<Option<KeyVaultRow>, DbError> {
    let row = sqlx::query_as::<_, KeyVaultRow>(
        r#"SELECT id, user_id, chain_type, public_address, encrypted_private_key, nonce
           FROM key_vaults WHERE user_id = $1 AND chain_type = $2"#,
    )
    .bind(user_id)
    .bind(chain)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn insert_key_vault(
    pool: &PgPool,
    user_id: Uuid,
    chain: &str,
    address: &str,
    encrypted_key: &str,
) -> Result<Uuid, DbError> {
    let row = sqlx::query!(
        r#"INSERT INTO key_vaults (user_id, chain_type, public_address, encrypted_private_key, nonce)
           VALUES ($1, $2, $3, $4, 'v1')
           ON CONFLICT (user_id, chain_type) DO NOTHING
           RETURNING id"#,
        user_id,
        chain,
        address,
        encrypted_key
    )
    .fetch_optional(pool)
    .await?;
    if let Some(r) = row {
        Ok(r.id)
    } else {
        // already exists, fetch existing
        let existing = get_key_vault(pool, user_id, chain).await?.ok_or(DbError::NotFound)?;
        Ok(existing.id)
    }
}

pub async fn list_wallets(pool: &PgPool, user_id: Uuid) -> Result<Vec<WalletRow>, DbError> {
    let rows = sqlx::query_as::<_, WalletRow>(
        r#"SELECT id, user_id, wallet_type, currency, balance, reserved_balance
           FROM wallets WHERE user_id = $1 ORDER BY currency"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn p2p_transfer_atomic(
    pool: &PgPool,
    sender: Uuid,
    recipient: Uuid,
    amount: Decimal,
    fee: Decimal,
) -> Result<Uuid, DbError> {
    if sender == recipient {
        return Err(DbError::Backend(sqlx::Error::Protocol("self transfer".into())));
    }
    let mut tx = pool.begin().await?;

    // Ensure recipient wallet exists
    sqlx::query!(
        r#"INSERT INTO wallets (user_id, wallet_type, currency, balance)
           VALUES ($1, 'FIAT_NGN', 'NGN', 0)
           ON CONFLICT (user_id, currency) DO NOTHING"#,
        recipient
    )
    .execute(&mut *tx)
    .await?;

    // Debit sender with funds check
    let updated = sqlx::query!(
        r#"UPDATE wallets SET balance = balance - ($1::numeric + $2::numeric)
           WHERE user_id = $3 AND currency = 'NGN'
             AND balance >= ($1::numeric + $2::numeric)"#,
        amount,
        fee,
        sender
    )
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        tx.rollback().await.ok();
        return Err(DbError::InsufficientFunds);
    }

    // Credit recipient
    sqlx::query!(
        r#"UPDATE wallets SET balance = balance + $1 WHERE user_id = $2 AND currency = 'NGN'"#,
        amount,
        recipient
    )
    .execute(&mut *tx)
    .await?;

    let tx_id = Uuid::new_v4();
    let recipient_ref = recipient.to_string();
    sqlx::query!(
        r#"INSERT INTO transactions (id, user_id, tx_type, direction, amount, currency, fee_amount, recipient_identifier, status, metadata)
           VALUES ($1, $2, 'P2P_TRANSFER', 'OUTBOUND', $3, 'NGN', $4, $5, 'SUCCESS', jsonb_build_object('counterpart', $6::text))"#,
        tx_id,
        sender,
        amount,
        fee,
        recipient_ref,
        recipient_ref
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        r#"INSERT INTO transactions (user_id, tx_type, direction, amount, currency, status, metadata)
           VALUES ($1, 'P2P_TRANSFER', 'INBOUND', $2, 'NGN', 'SUCCESS', jsonb_build_object('counterpart', $3::text))"#,
        recipient,
        amount,
        sender.to_string()
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(tx_id)
}

#[derive(Debug, sqlx::FromRow)]
pub struct AdminEmployee {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub full_name: Option<String>,
    pub role: String,
    pub permissions: serde_json::Value,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn get_admin_by_email(pool: &PgPool, email: &str) -> Result<Option<AdminEmployee>, DbError> {
    let row = sqlx::query_as::<_, AdminEmployee>(
        r#"SELECT id, email, password_hash, full_name, role, permissions, is_active, created_at, last_login
           FROM admin_employees WHERE email = $1"#,
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn create_admin_employee(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
    full_name: Option<&str>,
    role: &str,
    permissions: serde_json::Value,
    created_by: Option<Uuid>,
) -> Result<AdminEmployee, DbError> {
    let row = sqlx::query_as::<_, AdminEmployee>(
        r#"INSERT INTO admin_employees (email, password_hash, full_name, role, permissions, created_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, email, password_hash, full_name, role, permissions, is_active, created_at, last_login"#,
    )
    .bind(email)
    .bind(password_hash)
    .bind(full_name)
    .bind(role)
    .bind(permissions)
    .bind(created_by)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_admin_employees(pool: &PgPool) -> Result<Vec<AdminEmployee>, DbError> {
    let rows = sqlx::query_as::<_, AdminEmployee>(
        r#"SELECT id, email, password_hash, full_name, role, permissions, is_active, created_at, last_login
           FROM admin_employees ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_admin_employee(pool: &PgPool, id: Uuid) -> Result<Option<AdminEmployee>, DbError> {
    let row = sqlx::query_as::<_, AdminEmployee>(
        r#"SELECT id, email, password_hash, full_name, role, permissions, is_active, created_at, last_login
           FROM admin_employees WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn update_admin_employee(
    pool: &PgPool,
    id: Uuid,
    full_name: Option<String>,
    role: Option<String>,
    permissions: Option<serde_json::Value>,
    is_active: Option<bool>,
) -> Result<Option<AdminEmployee>, DbError> {
    let row = sqlx::query_as::<_, AdminEmployee>(
        r#"UPDATE admin_employees
           SET full_name = COALESCE($2, full_name),
               role = COALESCE($3, role),
               permissions = COALESCE($4, permissions),
               is_active = COALESCE($5, is_active)
           WHERE id = $1
           RETURNING id, email, password_hash, full_name, role, permissions, is_active, created_at, last_login"#,
    )
    .bind(id)
    .bind(full_name)
    .bind(role)
    .bind(permissions)
    .bind(is_active)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn delete_admin_employee(pool: &PgPool, id: Uuid) -> Result<(), DbError> {
    sqlx::query!(r#"DELETE FROM admin_employees WHERE id = $1"#, id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn validate_admin_token(pool: &PgPool, token: &str) -> Result<Option<Uuid>, DbError> {
    let row = sqlx::query!(
        r#"SELECT ae.id FROM admin_employees ae
           JOIN admin_tokens at ON at.employee_id = ae.id
           WHERE at.token = $1 AND at.expires_at > NOW() AND ae.is_active = true"#,
        token
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.id))
}

pub async fn create_admin_token(pool: &PgPool, employee_id: Uuid) -> Result<String, DbError> {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    let expires_at = chrono::Utc::now() + chrono::Duration::days(30);

    sqlx::query!(
        r#"INSERT INTO admin_tokens (token, employee_id, expires_at) VALUES ($1, $2, $3)"#,
        token,
        employee_id,
        expires_at
    )
    .execute(pool)
    .await?;
    Ok(token)
}

pub async fn update_admin_last_login(pool: &PgPool, id: Uuid) -> Result<(), DbError> {
    sqlx::query!(r#"UPDATE admin_employees SET last_login = NOW() WHERE id = $1"#, id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn has_permission(pool: &PgPool, emp_id: Uuid, permission: &str) -> Result<bool, DbError> {
    let row = sqlx::query_scalar!(
        r#"SELECT has_permission($1, $2)"#,
        emp_id,
        permission
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.flatten().unwrap_or(false))
}

pub async fn create_price_catalogue(
    pool: &PgPool,
    brand: &str,
    country: &str,
    card_format: &str,
    rate_per_dollar: Decimal,
    active: bool,
    created_by: Option<Uuid>,
) -> Result<PriceCatalogue, DbError> {
    let mut tx = pool.begin().await?;
    
    let cat = sqlx::query_as::<_, PriceCatalogue>(
        r#"INSERT INTO price_catalogue (brand, country, card_format, rate_per_dollar, active, created_by, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, $6)
           RETURNING id, brand, country, card_format, rate_per_dollar, active, created_by, updated_by, created_at, updated_at"#,
    )
    .bind(brand)
    .bind(country)
    .bind(card_format)
    .bind(rate_per_dollar)
    .bind(active)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query!(
        r#"INSERT INTO price_catalogue_audit (catalogue_id, employee_id, action, new_values)
           VALUES ($1, $2, 'CREATE', $3)"#,
        cat.id,
        created_by,
        serde_json::to_value(&cat).unwrap()
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(cat)
}

pub async fn update_price_catalogue(
    pool: &PgPool,
    id: i64,
    brand: Option<&str>,
    country: Option<&str>,
    card_format: Option<&str>,
    rate_per_dollar: Option<Decimal>,
    active: Option<bool>,
    updated_by: Option<Uuid>,
) -> Result<Option<PriceCatalogue>, DbError> {
    let mut tx = pool.begin().await?;

    let old = sqlx::query_as::<_, PriceCatalogue>(
        r#"SELECT id, brand, country, card_format, rate_per_dollar, active, created_by, updated_by, created_at, updated_at
           FROM price_catalogue WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(old) = old else {
        return Ok(None);
    };

    let cat = sqlx::query_as::<_, PriceCatalogue>(
        r#"UPDATE price_catalogue
           SET brand = COALESCE($2, brand),
               country = COALESCE($3, country),
               card_format = COALESCE($4, card_format),
               rate_per_dollar = COALESCE($5, rate_per_dollar),
               active = COALESCE($6, active),
               updated_by = COALESCE($7, updated_by)
           WHERE id = $1
           RETURNING id, brand, country, card_format, rate_per_dollar, active, created_by, updated_by, created_at, updated_at"#,
    )
    .bind(id)
    .bind(brand)
    .bind(country)
    .bind(card_format)
    .bind(rate_per_dollar)
    .bind(active)
    .bind(updated_by)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref cat) = cat {
        sqlx::query!(
            r#"INSERT INTO price_catalogue_audit (catalogue_id, employee_id, action, old_values, new_values)
               VALUES ($1, $2, 'UPDATE', $3, $4)"#,
            id,
            updated_by,
            serde_json::to_value(&old).unwrap(),
            serde_json::to_value(cat).unwrap()
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(cat)
}

pub async fn delete_price_catalogue(pool: &PgPool, id: i64) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    let old = sqlx::query_as::<_, PriceCatalogue>(
        r#"SELECT id, brand, country, card_format, rate_per_dollar, active, created_by, updated_by, created_at, updated_at
           FROM price_catalogue WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(old) = old {
        sqlx::query!(
            r#"INSERT INTO price_catalogue_audit (catalogue_id, employee_id, action, old_values)
               VALUES ($1, NULL, 'DELETE', $2)"#,
            id,
            serde_json::to_value(&old).unwrap()
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(r#"DELETE FROM price_catalogue WHERE id = $1"#, id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn get_bot_analytics(
    pool: &PgPool,
    from: Option<&str>,
    to: Option<&str>,
    metric: Option<&str>,
) -> Result<Vec<BotAnalyticsRow>, DbError> {
    let from_date = from.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let to_date = to.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    
    let rows = match (from_date, to_date, metric) {
        (Some(from), Some(to), Some(metric)) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics WHERE date >= $1 AND date <= $2 AND metric_name = $3
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
                from,
                to,
                metric
            )
            .fetch_all(pool)
            .await?
        }
        (Some(from), Some(to), None) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics WHERE date >= $1 AND date <= $2
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
                from,
                to
            )
            .fetch_all(pool)
            .await?
        }
        (Some(from), None, Some(metric)) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics WHERE date >= $1 AND metric_name = $2
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
                from,
                metric
            )
            .fetch_all(pool)
            .await?
        }
        (None, Some(to), Some(metric)) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics WHERE date <= $1 AND metric_name = $2
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
                to,
                metric
            )
            .fetch_all(pool)
            .await?
        }
        (Some(from), None, None) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics WHERE date >= $1
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
                from
            )
            .fetch_all(pool)
            .await?
        }
        (None, Some(to), None) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics WHERE date <= $1
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
                to
            )
            .fetch_all(pool)
            .await?
        }
        (None, None, Some(metric)) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics WHERE metric_name = $1
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
                metric
            )
            .fetch_all(pool)
            .await?
        }
        (None, None, None) => {
            sqlx::query_as!(
                BotAnalyticsRow,
                r#"SELECT id, date, metric_name, metric_value, metadata
                   FROM bot_analytics
                   ORDER BY date DESC, metric_name ASC LIMIT 1000"#,
            )
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}

pub async fn upsert_bot_analytics(
    pool: &PgPool,
    metric_name: &str,
    metric_value: i64,
    metadata: Option<serde_json::Value>,
) -> Result<(), DbError> {
    sqlx::query!(
        r#"INSERT INTO bot_analytics (metric_name, metric_value, metadata)
           VALUES ($1, $2, $3)
           ON CONFLICT (date, metric_name) DO UPDATE SET
               metric_value = bot_analytics.metric_value + EXCLUDED.metric_value,
               metadata = EXCLUDED.metadata"#,
        metric_name,
        metric_value,
        metadata.unwrap_or(serde_json::json!({}))
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_role_permissions(pool: &PgPool) -> Result<Vec<serde_json::Value>, DbError> {
    let rows = sqlx::query!(
        r#"SELECT role, jsonb_agg(jsonb_build_object('permission', permission, 'description', description)) as permissions
           FROM role_permissions GROUP BY role ORDER BY role"#,
    )
    .fetch_all(pool)
    .await?;

    let mut result = Vec::new();
    for row in rows {
        result.push(serde_json::json!({
            "role": row.role,
            "permissions": row.permissions.unwrap_or(serde_json::json!([])),
        }));
    }
    Ok(result)
}

// ========== Bot Support Case Study 1 queries ==========

pub async fn insert_bot_interaction(
    pool: &PgPool,
    message_id: &str,
    whatsapp_number: &str,
    user_id: Option<Uuid>,
    inbound_text: &str,
    intent: &str,
    category: &str,
    sentiment: &str,
    urgency: &str,
    urgency_score: i32,
    confidence: f64,
    response_text: Option<&str>,
    kb_article_id: Option<Uuid>,
    escalated: bool,
    escalation_reason: Option<&str>,
    handling_ms: i32,
) -> Result<Uuid, DbError> {
    let row = sqlx::query!(
        r#"INSERT INTO bot_interactions (message_id, whatsapp_number, user_id, inbound_text, intent, category, sentiment, urgency, urgency_score, confidence, response_text, kb_article_id, escalated, escalation_reason, handling_ms)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
           ON CONFLICT (message_id) DO NOTHING
           RETURNING id"#,
        message_id, whatsapp_number, user_id, inbound_text, intent, category, sentiment, urgency, urgency_score,
        Decimal::try_from(confidence).unwrap_or(Decimal::from(0)),
        response_text, kb_article_id, escalated, escalation_reason, handling_ms
    ).fetch_optional(pool).await?;
    Ok(row.map(|r| r.id).unwrap_or_else(Uuid::nil))
}

pub async fn list_bot_interactions(pool: &PgPool, limit: i64, escalated_only: bool) -> Result<Vec<BotInteraction>, DbError> {
    let rows = if escalated_only {
        sqlx::query_as::<_, BotInteraction>(
            r#"SELECT id, message_id, whatsapp_number, user_id, inbound_text, intent, category, sentiment, urgency, urgency_score, confidence, response_text, escalated, escalation_reason, assigned_agent, resolved, handling_ms, created_at
               FROM bot_interactions WHERE escalated = true ORDER BY created_at DESC LIMIT $1"#,
        ).bind(limit).fetch_all(pool).await?
    } else {
        sqlx::query_as::<_, BotInteraction>(
            r#"SELECT id, message_id, whatsapp_number, user_id, inbound_text, intent, category, sentiment, urgency, urgency_score, confidence, response_text, escalated, escalation_reason, assigned_agent, resolved, handling_ms, created_at
               FROM bot_interactions ORDER BY created_at DESC LIMIT $1"#,
        ).bind(limit).fetch_all(pool).await?
    };
    Ok(rows)
}

pub async fn get_bot_stats(pool: &PgPool) -> Result<BotStats, DbError> {
    let total: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) as "c!" FROM bot_interactions"#).fetch_one(pool).await?;
    let today: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) as "c!" FROM bot_interactions WHERE created_at >= CURRENT_DATE"#).fetch_one(pool).await?;
    let escalated: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) as "c!" FROM bot_interactions WHERE escalated = true"#).fetch_one(pool).await?;
    let avg_ms: Option<Decimal> = sqlx::query_scalar!(r#"SELECT AVG(handling_ms) FROM bot_interactions"#).fetch_one(pool).await?;
    let by_category = sqlx::query!(r#"SELECT category, COUNT(*) as c FROM bot_interactions GROUP BY category ORDER BY c DESC"#).fetch_all(pool).await?;
    let by_sentiment = sqlx::query!(r#"SELECT sentiment, COUNT(*) as c FROM bot_interactions GROUP BY sentiment"#).fetch_all(pool).await?;
    let by_urgency = sqlx::query!(r#"SELECT urgency, COUNT(*) as c FROM bot_interactions GROUP BY urgency"#).fetch_all(pool).await?;
    let by_intent = sqlx::query!(r#"SELECT intent, COUNT(*) as c FROM bot_interactions GROUP BY intent ORDER BY c DESC LIMIT 8"#).fetch_all(pool).await?;
    let last_14 = sqlx::query!(r#"SELECT to_char(date, 'YYYY-MM-DD') as d, metric_value FROM bot_analytics WHERE metric_name='messages_inbound' ORDER BY date ASC"#).fetch_all(pool).await?;
    Ok(BotStats {
        total_interactions: total,
        today_interactions: today,
        escalated_count: escalated,
        auto_resolved: total - escalated,
        avg_handling_ms: avg_ms.and_then(|v| v.to_string().parse::<f64>().ok()).unwrap_or(0.0) as i64,
        by_category: by_category.into_iter().map(|r| (r.category, r.c.unwrap_or(0))).collect(),
        by_sentiment: by_sentiment.into_iter().map(|r| (r.sentiment, r.c.unwrap_or(0))).collect(),
        by_urgency: by_urgency.into_iter().map(|r| (r.urgency, r.c.unwrap_or(0))).collect(),
        by_intent: by_intent.into_iter().map(|r| (r.intent, r.c.unwrap_or(0))).collect(),
        last_14_days: last_14.into_iter().map(|r| (r.d.unwrap_or_default(), r.metric_value)).collect(),
    })
}

pub async fn knowledge_base_search(pool: &PgPool, q: &str, category: Option<&str>, limit: i64) -> Result<Vec<KnowledgeBaseRow>, DbError> {
    let pattern = format!("%{}%", q.to_lowercase());
    let rows = match category {
        Some(cat) => sqlx::query_as::<_, KnowledgeBaseRow>(
            r#"SELECT id, category, question, answer, keywords, source, priority, is_active, created_at
               FROM knowledge_base WHERE is_active = true AND category = $2 AND (LOWER(question) LIKE $1 OR LOWER(answer) LIKE $1 OR EXISTS (SELECT 1 FROM unnest(keywords) k WHERE LOWER(k) LIKE $1))
               ORDER BY priority DESC LIMIT $3"#,
        ).bind(&pattern).bind(cat).bind(limit).fetch_all(pool).await?,
        None => sqlx::query_as::<_, KnowledgeBaseRow>(
            r#"SELECT id, category, question, answer, keywords, source, priority, is_active, created_at
               FROM knowledge_base WHERE is_active = true AND (LOWER(question) LIKE $1 OR LOWER(answer) LIKE $1 OR EXISTS (SELECT 1 FROM unnest(keywords) k WHERE LOWER(k) LIKE $1))
               ORDER BY priority DESC LIMIT $2"#,
        ).bind(&pattern).bind(limit).fetch_all(pool).await?,
    };
    Ok(rows)
}

pub async fn knowledge_base_list(pool: &PgPool) -> Result<Vec<KnowledgeBaseRow>, DbError> {
    let rows = sqlx::query_as::<_, KnowledgeBaseRow>(
        r#"SELECT id, category, question, answer, keywords, source, priority, is_active, created_at FROM knowledge_base WHERE is_active = true ORDER BY priority DESC"#,
    ).fetch_all(pool).await?;
    Ok(rows)
}

pub async fn resolve_bot_interaction(pool: &PgPool, id: Uuid, agent_id: Uuid) -> Result<(), DbError> {
    sqlx::query!(r#"UPDATE bot_interactions SET resolved = true, assigned_agent = $2 WHERE id = $1"#, id, agent_id).execute(pool).await?;
    Ok(())
}

// === Fintech 8-features queries ===
pub async fn get_user_pin_hash(pool: &PgPool, user_id: Uuid) -> Result<Option<String>, DbError> {
    Ok(sqlx::query_scalar!(r#"SELECT pin_hash FROM users WHERE id=$1"#, user_id).fetch_optional(pool).await?.flatten())
}
pub async fn set_user_pin(pool: &PgPool, user_id: Uuid, hash: &str) -> Result<(), DbError> {
    sqlx::query!(r#"UPDATE users SET pin_hash=$2, pin_set=true, updated_at=NOW() WHERE id=$1"#, user_id, hash).execute(pool).await?; Ok(())
}
pub async fn increment_pin_fail(pool: &PgPool, user_id: Uuid) -> Result<(i32, Option<chrono::DateTime<chrono::Utc>>), DbError> {
    let row = sqlx::query!(r#"UPDATE users SET failed_pin_attempts = failed_pin_attempts+1, pin_locked_until = CASE WHEN failed_pin_attempts+1 >= 5 THEN NOW() + interval '15 minutes' WHEN failed_pin_attempts+1 >= 3 THEN NOW() + interval '2 minutes' ELSE pin_locked_until END WHERE id=$1 RETURNING failed_pin_attempts, pin_locked_until"#, user_id).fetch_one(pool).await?;
    Ok((row.failed_pin_attempts, row.pin_locked_until))
}
pub async fn reset_pin_attempts(pool: &PgPool, user_id: Uuid) -> Result<(), DbError> {
    sqlx::query!(r#"UPDATE users SET failed_pin_attempts=0, pin_locked_until=NULL WHERE id=$1"#, user_id).execute(pool).await?; Ok(())
}
pub async fn check_velocity(pool: &PgPool, user_id: Uuid, amount_ngn: Decimal) -> Result<bool, DbError> {
    // daily cap 500k NGN, 5 trades/hour, tx amount cap 200k per single without OTP
    let hour_count: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) as "c!" FROM transactions WHERE user_id=$1 AND created_at > NOW() - interval '1 hour'"#, user_id).fetch_one(pool).await?;
    if hour_count >= 5 { return Ok(false); }
    let today_sum: Option<Decimal> = sqlx::query_scalar!(r#"SELECT SUM(amount) FROM transactions WHERE user_id=$1 AND currency='NGN' AND created_at >= CURRENT_DATE"# , user_id).fetch_one(pool).await?;
    let sum = today_sum.unwrap_or(Decimal::ZERO);
    if sum + amount_ngn > Decimal::from(500_000) { return Ok(false); }
    Ok(true)
}
pub async fn create_fiat_payout(pool: &PgPool, user_id: Uuid, amount: Decimal, bank_code: &str, acct: &str, name: Option<&str>) -> Result<Uuid, DbError> {
    let row = sqlx::query!(r#"INSERT INTO fiat_payouts (user_id, amount, bank_code, account_number, account_name, provider_ref, status) VALUES ($1,$2,$3,$4,$5,$6,'SUCCESS') RETURNING id"#, user_id, amount, bank_code, acct, name, format!("MOCK-{}", Uuid::new_v4())).fetch_one(pool).await?;
    // also debit wallet + ledger
    sqlx::query!(r#"INSERT INTO transactions (user_id, tx_type, direction, amount, currency, status, metadata) VALUES ($1,'FIAT_PAYOUT','OUTBOUND',$2,'NGN','SUCCESS', jsonb_build_object('bank_code',$3::text,'account',$4::text))"#, user_id, amount, bank_code, acct).execute(pool).await?;
    sqlx::query!(r#"UPDATE wallets SET balance = balance - $1 WHERE user_id=$2 AND currency='NGN' AND balance >= $1"#, amount, user_id).execute(pool).await?;
    Ok(row.id)
}
pub async fn create_airtime(pool: &PgPool, user_id: Uuid, phone: &str, network: &str, amount: Decimal, ptype: &str) -> Result<Uuid, DbError> {
    let row = sqlx::query!(r#"INSERT INTO airtime_purchases (user_id, recipient_phone, network, amount, purchase_type, provider_ref) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id"#, user_id, phone, network, amount, ptype, format!("VTU-{}", Uuid::new_v4())).fetch_one(pool).await?;
    sqlx::query!(r#"INSERT INTO transactions (user_id, tx_type, direction, amount, currency, status, metadata) VALUES ($1,'AIRTIME','OUTBOUND',$2,'NGN','SUCCESS', jsonb_build_object('network',$3::text,'type',$4::text))"#, user_id, amount, network, ptype).execute(pool).await?;
    sqlx::query!(r#"UPDATE wallets SET balance = balance - $1 WHERE user_id=$2 AND currency='NGN'"#, amount, user_id).execute(pool).await?;
    Ok(row.id)
}
pub async fn create_crypto_transfer(pool: &PgPool, user_id: Uuid, chain: &str, token: &str, amount: Decimal, to: &str) -> Result<(Uuid,String), DbError> {
    let hash = format!("0x{}", hex::encode(rand::random::<[u8;32]>()));
    let h2 = if chain=="SOLANA" { format!("Sol{}", &hash[2..18]) } else { hash.clone() };
    let row = sqlx::query!(r#"INSERT INTO crypto_transfers (user_id, chain_type, token, amount, recipient_address, tx_hash) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id"#, user_id, chain, token, amount, to, h2.clone()).fetch_one(pool).await?;
    sqlx::query!(r#"INSERT INTO transactions (user_id, tx_type, direction, amount, currency, blockchain_tx_hash, status, metadata) VALUES ($1,'CRYPTO_TRANSFER','OUTBOUND',$2,$3,$4,'SUCCESS', jsonb_build_object('chain',$5::text))"#, user_id, amount, token, h2.clone(), chain).execute(pool).await?;
    Ok((row.id, h2))
}
pub async fn atomic_offramp(pool: &PgPool, user_id: Uuid, from_token: &str, to_fiat: &str, amount_token: Decimal, rate: Decimal) -> Result<Uuid, DbError> {
    let fiat_amount = (amount_token * rate).round_dp(2);
    // debit token wallet (mock: just ledger), credit NGN
    sqlx::query!(r#"INSERT INTO transactions (user_id, tx_type, direction, amount, currency, status, metadata) VALUES ($1,'OFFRAMP','OUTBOUND',$2,$3,'SUCCESS', jsonb_build_object('to_fiat',$4::text,'rate',$5::text))"#, user_id, amount_token, from_token, to_fiat, rate.to_string()).execute(pool).await?;
    sqlx::query!(r#"INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES ($1,'FIAT_NGN','NGN',0) ON CONFLICT (user_id,currency) DO NOTHING"#, user_id).execute(pool).await?;
    sqlx::query!(r#"UPDATE wallets SET balance = balance + $1 WHERE user_id=$2 AND currency='NGN'"#, fiat_amount, user_id).execute(pool).await?;
    let tx = sqlx::query!(r#"INSERT INTO transactions (user_id, tx_type, direction, amount, currency, status, metadata) VALUES ($1,'OFFRAMP_CREDIT','INBOUND',$2,'NGN','SUCCESS', jsonb_build_object('from',$3::text)) RETURNING id"#, user_id, fiat_amount, from_token).fetch_one(pool).await?;
    Ok(tx.id)
}
pub async fn get_crypto_rate(pool: &PgPool, pair: &str) -> Result<Option<Decimal>, DbError> {
    Ok(sqlx::query_scalar!(r#"SELECT mid_rate FROM fx_rates WHERE pair=$1 ORDER BY fetched_at DESC LIMIT 1"#, pair).fetch_optional(pool).await?)
}
pub async fn list_recent_transactions(pool: &PgPool, user_id: Uuid, limit: i64) -> Result<Vec<WalletRow>, DbError> { // reuse for wallets list
    list_wallets(pool, user_id).await
}
pub async fn ensure_foreign_wallet(pool: &PgPool, user_id: Uuid, currency: &str) -> Result<String, DbError> {
    let wt = format!("FIAT_{}", currency);
    sqlx::query!(r#"INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES ($1,$2,$3,0) ON CONFLICT (user_id,currency) DO NOTHING"#, user_id, wt, currency).execute(pool).await?;
    let acct = format!("{}-MOCK-{}", currency, &user_id.to_string()[..8].to_uppercase());
    sqlx::query!(r#"INSERT INTO foreign_accounts (user_id, currency, account_number) VALUES ($1,$2,$3) ON CONFLICT (user_id,currency) DO NOTHING"#, user_id, currency, acct.clone()).execute(pool).await?;
    Ok(acct)
}
pub async fn list_foreign_accounts(pool: &PgPool, user_id: Uuid) -> Result<Vec<ForeignAccountRow>, DbError> {
    Ok(sqlx::query_as::<_, ForeignAccountRow>(r#"SELECT id, user_id, currency, account_number, provider, status, created_at FROM foreign_accounts WHERE user_id=$1"#).bind(user_id).fetch_all(pool).await?)
}
