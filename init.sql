CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- 1. Core Users Table
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    whatsapp_number VARCHAR(20) UNIQUE NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- 2. Fiat Wallets (For Naira balances, Palmpay, and Airtime-to-Cash)
CREATE TABLE fiat_wallets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    currency VARCHAR(10) DEFAULT 'NGN',
    balance DECIMAL(15, 2) DEFAULT 0.00,
    UNIQUE(user_id, currency)
);

-- 3. CEX Wallets (Your Internal Crypto Ledger)
CREATE TABLE cex_wallets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    asset VARCHAR(10) NOT NULL, -- e.g., 'USDT', 'BTC'
    balance DECIMAL(18, 8) DEFAULT 0.00000000,
    UNIQUE(user_id, asset)
);

-- 4. DEX Wallets (On-Chain Wallets across Networks)
CREATE TABLE dex_wallets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    network VARCHAR(20) NOT NULL, -- e.g., 'EVM', 'SOLANA', 'TON'
    public_address VARCHAR(100) UNIQUE NOT NULL,
    encrypted_private_key TEXT NOT NULL, 
    UNIQUE(user_id, network)
);

-- 5. Gift Card Trades Queue (Manual Processing)
CREATE TABLE gift_card_trades (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    card_category VARCHAR(50) NOT NULL, 
    country VARCHAR(10) NOT NULL, 
    card_type VARCHAR(20) NOT NULL, -- e.g., 'physical' or 'ecode'
    amount_in_usd DECIMAL(10, 2) NOT NULL,
    offered_naira_value DECIMAL(15, 2) NOT NULL,
    e_code_data TEXT, 
    image_url TEXT, 
    status VARCHAR(20) DEFAULT 'pending', -- 'pending', 'approved', 'rejected'
    agent_notes TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- 6. Master Transaction Ledger (Crucial for FinTech Auditing)
CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    type VARCHAR(50) NOT NULL, -- e.g., 'PALMPAY_WITHDRAWAL', 'GIFTCARD_CREDIT', 'AIRTIME_PURCHASE'
    amount DECIMAL(18, 8) NOT NULL,
    currency VARCHAR(10) NOT NULL,
    reference VARCHAR(100) UNIQUE,
    status VARCHAR(20) DEFAULT 'PENDING',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);