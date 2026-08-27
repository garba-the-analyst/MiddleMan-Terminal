CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    whatsapp_number VARCHAR(32) NOT NULL UNIQUE,
    full_name VARCHAR(128),
    pin_hash VARCHAR(255),
    pin_salt VARCHAR(64),
    failed_pin_attempts INT NOT NULL DEFAULT 0,
    pin_locked_until TIMESTAMPTZ,
    current_state VARCHAR(64) NOT NULL DEFAULT 'IDLE',
    state_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    kyc_status VARCHAR(32) NOT NULL DEFAULT 'UNVERIFIED',
    bvn_nin_hash VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_users_whatsapp ON users(whatsapp_number);
CREATE INDEX idx_users_state ON users(current_state);
CREATE TRIGGER trg_users_updated BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE wallets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    wallet_type VARCHAR(32) NOT NULL,
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

CREATE TABLE admin_employees (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email VARCHAR(128) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    role VARCHAR(32) NOT NULL DEFAULT 'AGENT',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT ck_admin_role CHECK (role IN ('SUPER_ADMIN','AGENT','COMPLIANCE'))
);

CREATE TABLE gift_card_trades (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    card_brand VARCHAR(64) NOT NULL,
    country VARCHAR(8) NOT NULL,
    card_format VARCHAR(16) NOT NULL,
    claimed_usd_amount NUMERIC(12,2) NOT NULL,
    offered_ngn_rate NUMERIC(12,2) NOT NULL,
    final_ngn_payout NUMERIC(12,2) NOT NULL,
    extracted_code VARCHAR(255),
    image_url TEXT,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    rejection_reason TEXT,
    reviewed_by_employee_id UUID REFERENCES admin_employees(id) ON DELETE SET NULL,
    message_id VARCHAR(64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_gift_card_trades_status ON gift_card_trades(status);
CREATE INDEX idx_gift_card_trades_user ON gift_card_trades(user_id);

CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tx_type VARCHAR(64) NOT NULL,
    direction VARCHAR(8) NOT NULL,
    amount NUMERIC(36,18) NOT NULL,
    currency VARCHAR(16) NOT NULL,
    fee_amount NUMERIC(36,18) NOT NULL DEFAULT 0,
    recipient_identifier VARCHAR(128),
    blockchain_tx_hash VARCHAR(128),
    status VARCHAR(32) NOT NULL DEFAULT 'PROCESSING',
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT ck_direction CHECK (direction IN ('INBOUND','OUTBOUND')),
    CONSTRAINT ck_tx_status CHECK (status IN ('PROCESSING','SUCCESS','FAILED'))
);
CREATE INDEX idx_transactions_user ON transactions(user_id);
CREATE INDEX idx_transactions_hash ON transactions(blockchain_tx_hash);
CREATE INDEX idx_transactions_created ON transactions(created_at DESC);

CREATE TABLE active_positions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    protocol VARCHAR(32) NOT NULL,
    market_pair VARCHAR(32) NOT NULL,
    side VARCHAR(8) NOT NULL,
    leverage NUMERIC(5,2) NOT NULL,
    margin_usd NUMERIC(18,4) NOT NULL,
    entry_price NUMERIC(18,4) NOT NULL,
    liquidation_price NUMERIC(18,4) NOT NULL,
    take_profit_price NUMERIC(18,4),
    stop_loss_price NUMERIC(18,4),
    status VARCHAR(16) NOT NULL DEFAULT 'OPEN',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_positions_user ON active_positions(user_id);
CREATE INDEX idx_positions_status ON active_positions(status);

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

CREATE TABLE key_vaults (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    chain_type VARCHAR(32) NOT NULL,
    public_address VARCHAR(128) NOT NULL,
    encrypted_private_key TEXT NOT NULL,
    nonce VARCHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uk_user_chain UNIQUE(user_id, chain_type)
);
CREATE INDEX idx_key_vaults_user ON key_vaults(user_id);
CREATE INDEX idx_key_vaults_address ON key_vaults(public_address);

CREATE TABLE processed_messages (
    message_id VARCHAR(64) PRIMARY KEY,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE processed_flw_events (
    tx_ref VARCHAR(128) PRIMARY KEY,
    flw_id VARCHAR(32),
    credited_amount_ngn NUMERIC(18,2) NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE price_catalogue (
    id BIGSERIAL PRIMARY KEY,
    brand VARCHAR(64) NOT NULL,
    country VARCHAR(8) NOT NULL DEFAULT 'US',
    card_format VARCHAR(16) NOT NULL DEFAULT 'PHYSICAL',
    rate_per_dollar NUMERIC(12,2) NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_catalogue_lookup ON price_catalogue(brand, country, card_format, active);

CREATE TABLE fx_rates (
    id BIGSERIAL PRIMARY KEY,
    pair VARCHAR(16) NOT NULL,
    mid_rate NUMERIC(20,8) NOT NULL,
    source VARCHAR(32) NOT NULL,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_fx_rates_pair_time ON fx_rates(pair, fetched_at DESC);
