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
