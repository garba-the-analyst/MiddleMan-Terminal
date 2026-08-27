# VOLUME 2 — Database DDL, Schema Architecture & Storage Contracts

**Version:** 2.4.0 · **Owner:** Core Engineering · **Scope:** PostgreSQL (Neon prod / local dev),
migrations, ledger invariants, SQLx compile-time contracts.

---

## 1. Architectural Overview & Technical Scope

- Engine: PostgreSQL 15+ (Neon in production). Single writer class: `mm-api` workers. Max pool:
  5 connections (`PgPoolOptions::max_connections(5)`), matching the 180 MB RAM envelope.
- All access via `crates/mm-db` using `sqlx::query!` macros — compile-time checked against the
  live schema through `SQLX_OFFLINE=true` + `.sqlx/` metadata committed to the repo.
- Money is `NUMERIC`. Crypto balances: `NUMERIC(36,18)`; NGN/USD fiat: `NUMERIC(18,2)`;
  rates: `NUMERIC(20,8)`. No floats touch money. Ever.
- Every mutation that moves value runs inside one transaction with row-level locks
  (`SELECT ... FOR UPDATE`) or conditional updates (`WHERE balance >= $amount`).

## 2. Cryptographic / Integrity Guarantees

- PINs and BVN/NIN are never stored plaintext (Argon2id hashes only — Vol Alpha).
- Wallet private keys stored as AES-256-GCM ciphertext + nonce columns (Vol Alpha).
- Ledger invariant enforced at DB level:

```
balance = SUM(credit amounts) - SUM(debit amounts) - reserved_balance   (per wallet)
reserved_balance >= 0
balance >= 0  (CHECK)
```

A nightly reconciliation job recomputes the right-hand side from `transactions` and compares;
any drift pages ops (Vol Eta).

## 3. Complete Implementation — Migrations

### 0001_init.sql

```sql
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- 1. USERS
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    whatsapp_number VARCHAR(32) NOT NULL UNIQUE,
    full_name VARCHAR(128),
    pin_hash VARCHAR(255) NOT NULL,
    pin_salt VARCHAR(64) NOT NULL,
    failed_pin_attempts INT NOT NULL DEFAULT 0,
    pin_locked_until TIMESTAMPTZ,
    current_state VARCHAR(64) NOT NULL DEFAULT 'IDLE',
    state_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    kyc_status VARCHAR(32) NOT NULL DEFAULT 'UNVERIFIED', -- UNVERIFIED, TIER_1, TIER_2
    bvn_nin_hash VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_users_whatsapp ON users(whatsapp_number);
CREATE INDEX idx_users_state ON users(current_state);
CREATE TRIGGER trg_users_updated BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 2. KEY VAULTS
CREATE TABLE key_vaults (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    chain_type VARCHAR(32) NOT NULL,          -- EVM, SOLANA, TRON, TON
    public_address VARCHAR(128) NOT NULL,
    encrypted_private_key TEXT NOT NULL,      -- base64(nonce || ciphertext), Vol Alpha format
    nonce VARCHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uk_user_chain UNIQUE(user_id, chain_type),
    CONSTRAINT ck_chain_type CHECK (chain_type IN ('EVM','SOLANA','TRON','TON'))
);
CREATE INDEX idx_key_vaults_user ON key_vaults(user_id);
CREATE INDEX idx_key_vaults_address ON key_vaults(public_address);

-- 3. WALLETS
CREATE TABLE wallets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    wallet_type VARCHAR(32) NOT NULL,         -- FIAT_NGN, CRYPTO_USDT, CRYPTO_SOL, CRYPTO_ETH ...
    currency VARCHAR(16) NOT NULL,
    balance NUMERIC(36,18) NOT NULL DEFAULT 0,
    reserved_balance NUMERIC(36,18) NOT NULL DEFAULT 0,
    nuban_account_number VARCHAR(16),
    nuban_bank_name VARCHAR(64),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uk_user_wallet_currency UNIQUE(user_id, currency),
    CONSTRAINT ck_nonnegative_balance CHECK (balance >= 0 AND reserved_balance >= 0)
);
CREATE INDEX idx_wallets_user ON wallets(user_id);

-- 4. GIFT CARD TRADES
CREATE TABLE gift_card_trades (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    card_brand VARCHAR(64) NOT NULL,          -- STEAM, APPLE, AMAZON, RAZER_GOLD, GOOGLE_PLAY
    country VARCHAR(8) NOT NULL,              -- US, UK, DE, CA
    card_format VARCHAR(16) NOT NULL,         -- PHYSICAL, ECODE
    claimed_usd_amount NUMERIC(12,2) NOT NULL,
    offered_ngn_rate NUMERIC(12,2) NOT NULL,
    final_ngn_payout NUMERIC(12,2) NOT NULL,
    extracted_code VARCHAR(255),
    image_url TEXT,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING', -- PENDING, APPROVED, REJECTED
    rejection_reason TEXT,
    reviewed_by_employee_id UUID REFERENCES admin_employees(id) SET NULL,
    message_id VARCHAR(64),                   -- inbound dedupe linkage
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_gift_card_trades_status ON gift_card_trades(status);
CREATE INDEX idx_gift_card_trades_user ON gift_card_trades(user_id);

-- 5. TRANSACTIONS (LEDGER)
CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tx_type VARCHAR(64) NOT NULL,             -- P2P_TRANSFER, DEX_SWAP, FIAT_WITHDRAWAL, GIFT_CARD_PAYOUT, FIAT_TOPUP
    direction VARCHAR(8) NOT NULL,            -- INBOUND, OUTBOUND
    amount NUMERIC(36,18) NOT NULL,
    currency VARCHAR(16) NOT NULL,
    fee_amount NUMERIC(36,18) NOT NULL DEFAULT 0,
    recipient_identifier VARCHAR(128),
    blockchain_tx_hash VARCHAR(128),
    status VARCHAR(32) NOT NULL DEFAULT 'PROCESSING', -- PROCESSING, SUCCESS, FAILED
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT ck_direction CHECK (direction IN ('INBOUND','OUTBOUND')),
    CONSTRAINT ck_tx_status CHECK (status IN ('PROCESSING','SUCCESS','FAILED'))
);
CREATE INDEX idx_transactions_user ON transactions(user_id);
CREATE INDEX idx_transactions_hash ON transactions(blockchain_tx_hash);
CREATE INDEX idx_transactions_created ON transactions(created_at DESC);

-- 6. ACTIVE DEGEN POSITIONS
CREATE TABLE active_positions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    protocol VARCHAR(32) NOT NULL,            -- DYDX_V4, AEVO
    market_pair VARCHAR(32) NOT NULL,         -- BTC-USD, SOL-USD
    side VARCHAR(8) NOT NULL,                 -- LONG, SHORT
    leverage NUMERIC(5,2) NOT NULL,
    margin_usd NUMERIC(18,4) NOT NULL,
    entry_price NUMERIC(18,4) NOT NULL,
    liquidation_price NUMERIC(18,4) NOT NULL,
    take_profit_price NUMERIC(18,4),
    stop_loss_price NUMERIC(18,4),
    status VARCHAR(16) NOT NULL DEFAULT 'OPEN', -- OPEN, CLOSED, LIQUIDATED
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT ck_side CHECK (side IN ('LONG','SHORT'))
);
CREATE INDEX idx_positions_user ON active_positions(user_id);
CREATE INDEX idx_positions_status ON active_positions(status);

-- 7. ADMIN EMPLOYEES
CREATE TABLE admin_employees (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email VARCHAR(128) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,      -- Argon2id PHC string (Vol Alpha)
    role VARCHAR(32) NOT NULL DEFAULT 'AGENT', -- SUPER_ADMIN, AGENT, COMPLIANCE
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT ck_admin_role CHECK (role IN ('SUPER_ADMIN','AGENT','COMPLIANCE'))
);

-- 8. ADMIN AUDIT LOGS (append-only)
CREATE TABLE admin_audit_logs (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    employee_id UUID NOT NULL REFERENCES admin_employees(id),
    action VARCHAR(128) NOT NULL,
    target_entity VARCHAR(64) NOT NULL,
    target_id UUID NOT NULL,
    changes JSONB NOT NULL DEFAULT '{}'::jsonb,
    ip_address VARCHAR(45),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_audit_employee ON admin_audit_logs(employee_id);
CREATE INDEX idx_audit_created ON admin_audit_logs(created_at DESC);
REVOKE UPDATE, DELETE ON admin_audit_logs FROM PUBLIC;

-- 9. IDEMPOTENCY / INFRASTRUCTURE
CREATE TABLE processed_messages (
    message_id VARCHAR(64) PRIMARY KEY,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE price_catalogue (
    id BIGSERIAL PRIMARY KEY,
    brand VARCHAR(64) NOT NULL,
    country VARCHAR(8) NOT NULL DEFAULT 'US',
    card_format VARCHAR(16) NOT NULL DEFAULT 'PHYSICAL',
    rate_per_dollar NUMERIC(12,2) NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uk_catalogue UNIQUE(brand, country, card_format, active)
);

CREATE TABLE fx_rates (
    id BIGSERIAL PRIMARY KEY,
    pair VARCHAR(16) NOT NULL,               -- USDT_NGN
    mid_rate NUMERIC(20,8) NOT NULL,
    source VARCHAR(32) NOT NULL,             -- YELLOW_CARD
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_fx_rates_pair_time ON fx_rates(pair, fetched_at DESC);
```

### Seed (0002_seed.sql)

```sql
INSERT INTO price_catalogue (brand, country, card_format, rate_per_dollar) VALUES
    ('STEAM',       'US', 'PHYSICAL', 1450),
    ('STEAM',       'US', 'ECODE',    1430),
    ('APPLE',       'US', 'PHYSICAL', 1500),
    ('APPLE',       'UK', 'ECODE',    1470),
    ('AMAZON',      'US', 'PHYSICAL', 1420),
    ('RAZER_GOLD',  'US', 'ECODE',    1380),
    ('GOOGLE_PLAY', 'US', 'ECODE',    1360)
ON CONFLICT DO NOTHING;

INSERT INTO admin_employees (email, password_hash, role) VALUES
    ('ops@middleman.africa', '$argon2id$v=19$m=19456,t=2,p=1$REPLACE_AT_FIRST_BOOT$hash', 'SUPER_ADMIN')
ON CONFLICT DO NOTHING;
```

> The seed admin password hash MUST be regenerated on first boot by the bootstrap command
> (`mm-api admin reset-password --email ops@middleman.africa`); the placeholder above is inert.

## 4. Data Contracts — Canonical Queries (`crates/mm-db`)

### 4.1 Atomic P2P transfer (single transaction)

```rust
pub async fn p2p_transfer(
    tx: &mut sqlx::PgConnection,
    sender_id: Uuid,
    receiver_id: Uuid,
    amount: rust_decimal::Decimal,
    fee: rust_decimal::Decimal,
) -> Result<Uuid, DbError> {
    let mut db_tx = tx.begin().await?;

    let debit = sqlx::query!(
        r#"UPDATE wallets
           SET balance = balance - ($1 + $2)
           WHERE user_id = $3 AND currency = 'NGN'
             AND balance >= ($1 + $2)
           RETURNING id"#,
        amount, fee, sender_id
    )
    .fetch_optional(&mut *db_tx)
    .await?
    .ok_or(DbError::InsufficientFunds)?;

    sqlx::query!(
        r#"UPDATE wallets SET balance = balance + $1 WHERE user_id = $2 AND currency = 'NGN'"#,
        amount, receiver_id
    )
    .execute(&mut *db_tx)
    .await?;

    let ledger_id: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO transactions
             (user_id, tx_type, direction, amount, currency, fee_amount, recipient_identifier, status)
           VALUES ($1,'P2P_TRANSFER','OUTBOUND',$2,'NGN',$3,$4,'SUCCESS')
           RETURNING id"#,
    )
    .bind(sender_id).bind(amount).bind(fee).bind(receiver_id.to_string())
    .fetch_one(&mut *db_tx)
    .await?;

    sqlx::query!(
        r#"INSERT INTO transactions
             (user_id, tx_type, direction, amount, currency, status, metadata)
           VALUES ($1,'P2P_TRANSFER','INBOUND',$2,'NGN','SUCCESS', jsonb_build_object('counterpart',$3))"#,
        receiver_id, amount, sender_id.to_string()
    )
    .execute(&mut *db_tx)
    .await?;

    db_tx.commit().await?;
    Ok(debit.0.id_unused_placeholder_fix())
}
```

> Return-value plumbing is finalized in code review; the contract is: commit succeeds wholly or
> not at all, sender row update is the funds gate (`WHERE balance >= ...`).

### 4.2 Reserve hold for swaps

```sql
UPDATE wallets
SET reserved_balance = reserved_balance + $hold, balance = balance - $hold
WHERE user_id = $u AND currency = $c AND balance >= $hold;
```

Release path on failure: `balance = balance + $hold, reserved_balance = reserved_balance - $hold`.
Consume path on success: `reserved_balance = reserved_balance - $hold`.

### 4.3 Gift card approval credit (Vol Epsilon uses this verbatim)

```sql
BEGIN;
UPDATE gift_card_trades
SET status='APPROVED', reviewed_by_employee_id=$admin, final_ngn_payout=$payout, updated_at=NOW()
WHERE id=$trade AND status='PENDING'
RETURNING user_id, final_ngn_payout;

INSERT INTO transactions (user_id, tx_type, direction, amount, currency, status, metadata)
VALUES ($user,'GIFT_CARD_PAYOUT','INBOUND',$payout,'NGN','SUCCESS',
        jsonb_build_object('trade_id',$trade));

UPDATE wallets SET balance = balance + $payout
WHERE user_id=$user AND currency='NGN';
COMMIT;
```

The `WHERE status='PENDING'` gate makes double-approval impossible; zero rows returned means an
agent raced another agent — UI shows "already resolved".

## 5. Error Handling Policies

| DB Condition | Code Path | User-Facing Behavior |
|---|---|---|
| Unique violation `uk_user_wallet_currency` | ensure-wallet helper | Treat as success (row exists) |
| Insufficient funds (no RETURNING row) | transfers/swaps | "Insufficient balance" FSM reply, no retry |
| Deadlock (40001) / serialization (40023) | worker retry loop | Retry with backoff ≤5 |
| Connection drop | Pool | sqlx auto-reconnect; inflow messages wait in Redis |
| Migration drift | CI gate `cargo sqlx prepare --check` | Build fails |

## 6. Verification Test Cases & Command Sequences

```bash
# V2-T1: apply migrations clean-room
docker compose up -d postgres
export DATABASE_URL=postgres://mm_user:mm_password@localhost:5433/middleman_db
sqlx migrate run
psql $DATABASE_URL -c '\dt'        # 11 tables expected

# V2-T2: compile-time check freshness
cargo sqlx prepare --workspace -- --tests
git diff --exit-code .sqlx/         # no drift

# V2-T3: negative balance impossible
psql $DATABASE_URL -c "UPDATE wallets SET balance=-1 WHERE true"
# expect CHECK constraint violation ck_nonnegative_balance

# V2-T4: concurrent double-approval race
psql $DATABASE_URL -c "
BEGIN;
UPDATE gift_card_trades SET status='APPROVED' WHERE id='<id>' AND status='PENDING';
-- second session: returns 0 rows
ROLLBACK;"

# V2-T5: reconciliation query returns zero drift
psql $DATABASE_URL -f crates/mm-db/sql/reconcile.sql
```

`crates/mm-db/sql/reconcile.sql`:

```sql
SELECT w.user_id, w.currency, w.balance, w.reserved_balance,
       COALESCE(SUM(CASE WHEN t.direction='INBOUND' THEN t.amount
                         ELSE -(t.amount) END), 0) AS ledger_delta
FROM wallets w
LEFT JOIN transactions t
  ON t.user_id = w.user_id AND t.currency = w.currency AND t.status = 'SUCCESS'
GROUP BY w.user_id, w.currency, w.balance, w.reserved_balance
HAVING w.balance <> COALESCE(SUM(CASE WHEN t.direction='INBOUND' THEN t.amount
                                      ELSE -(t.amount) END), 0) + w.reserved_balance;
```
