-- 0006: Banking++ Crypto++ Security without UX friction

-- Extend wallets for foreign accounts (frontend: Foreign Accounts)
ALTER TABLE wallets DROP CONSTRAINT IF EXISTS uk_user_wallet_currency;
ALTER TABLE wallets ADD CONSTRAINT uk_user_wallet_currency UNIQUE(user_id, currency);

-- Users: add pin_set flag for first-time flow
ALTER TABLE users ADD COLUMN IF NOT EXISTS pin_set BOOLEAN DEFAULT false;
ALTER TABLE users ADD COLUMN IF NOT EXISTS daily_spent_ngn NUMERIC(18,2) DEFAULT 0;
ALTER TABLE users ADD COLUMN IF NOT EXISTS daily_spent_reset DATE DEFAULT CURRENT_DATE;

-- FX / Crypto rates (seeded)
INSERT INTO fx_rates (pair, mid_rate, source) VALUES
('USD/NGN', 1600.00, 'mock'), ('GBP/NGN', 2050.00, 'mock'), ('EUR/NGN', 1750.00, 'mock'),
('USDT/NGN', 1595.00, 'mock'), ('SOL/NGN', 85000.00, 'mock'), ('ETH/NGN', 4800000.00, 'mock')
ON CONFLICT DO NOTHING;

-- External fiat payouts (bank/fintech) – mock provider
CREATE TABLE IF NOT EXISTS fiat_payouts (
  id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  amount NUMERIC(36,18) NOT NULL,
  currency VARCHAR(16) NOT NULL DEFAULT 'NGN',
  bank_code VARCHAR(16) NOT NULL,
  account_number VARCHAR(32) NOT NULL,
  account_name VARCHAR(128),
  provider_ref VARCHAR(128),
  status VARCHAR(16) NOT NULL DEFAULT 'SUCCESS' CHECK (status IN ('PENDING','SUCCESS','FAILED')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_fiat_payouts_user ON fiat_payouts(user_id);

-- Airtime / Data / Utility purchases (mock VTU)
CREATE TABLE IF NOT EXISTS airtime_purchases (
  id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  recipient_phone VARCHAR(32) NOT NULL,
  network VARCHAR(16) NOT NULL,
  amount NUMERIC(36,18) NOT NULL,
  purchase_type VARCHAR(16) NOT NULL DEFAULT 'AIRTIME' CHECK (purchase_type IN ('AIRTIME','DATA','UTILITY')),
  status VARCHAR(16) NOT NULL DEFAULT 'SUCCESS',
  provider_ref VARCHAR(128),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_airtime_user ON airtime_purchases(user_id);

-- Crypto onchain transfers (mock signer)
CREATE TABLE IF NOT EXISTS crypto_transfers (
  id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  chain_type VARCHAR(16) NOT NULL CHECK (chain_type IN ('EVM','SOLANA')),
  token VARCHAR(16) NOT NULL,
  amount NUMERIC(36,18) NOT NULL,
  recipient_address VARCHAR(128) NOT NULL,
  tx_hash VARCHAR(128),
  status VARCHAR(16) NOT NULL DEFAULT 'SUCCESS',
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_crypto_user ON crypto_transfers(user_id);

-- Foreign virtual accounts (future)
CREATE TABLE IF NOT EXISTS foreign_accounts (
  id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  currency VARCHAR(8) NOT NULL CHECK (currency IN ('USD','GBP','EUR')),
  account_number VARCHAR(64) NOT NULL,
  provider VARCHAR(32) NOT NULL DEFAULT 'mock',
  status VARCHAR(16) NOT NULL DEFAULT 'ACTIVE',
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(user_id, currency)
);

-- OTP codes for step-up (>100k or new device)
CREATE TABLE IF NOT EXISTS otp_codes (
  id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
  whatsapp_number VARCHAR(32) NOT NULL,
  code VARCHAR(8) NOT NULL,
  purpose VARCHAR(32) NOT NULL DEFAULT 'STEP_UP',
  expires_at TIMESTAMPTZ NOT NULL,
  used BOOLEAN DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_otp_number ON otp_codes(whatsapp_number, expires_at);

-- Update CHECK constraints for new currencies
ALTER TABLE transactions DROP CONSTRAINT IF EXISTS ck_tx_status;
ALTER TABLE transactions ADD CONSTRAINT ck_tx_status CHECK (status IN ('PROCESSING','SUCCESS','FAILED','PENDING'));

-- Seed foreign wallets for demo user (2348012345678)
DO $$
DECLARE uid UUID;
BEGIN
  SELECT id INTO uid FROM users WHERE whatsapp_number='2348012345678';
  IF uid IS NOT NULL THEN
    INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES (uid,'FIAT_USD','USD',250.00) ON CONFLICT (user_id,currency) DO NOTHING;
    INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES (uid,'FIAT_GBP','GBP',80.00) ON CONFLICT (user_id,currency) DO NOTHING;
    INSERT INTO foreign_accounts (user_id, currency, account_number, provider) VALUES (uid,'USD','US64MOCK123456789','mock') ON CONFLICT DO NOTHING;
  END IF;
END $$;

-- Demo external payout + airtime + crypto to fill analytics
DO $$
DECLARE uid UUID; r TEXT;
BEGIN
  SELECT id INTO uid FROM users WHERE whatsapp_number='2348012345678';
  IF uid IS NOT NULL THEN
    INSERT INTO fiat_payouts (user_id, amount, currency, bank_code, account_number, account_name, provider_ref, status) VALUES (uid,50000,'NGN','044','0123456789','Garba Demo','MOCK-REF-1','SUCCESS');
    INSERT INTO airtime_purchases (user_id, recipient_phone, network, amount, purchase_type, provider_ref) VALUES (uid,'08012345678','MTN',1000,'AIRTIME','VTU-1');
    INSERT INTO airtime_purchases (user_id, recipient_phone, network, amount, purchase_type, provider_ref) VALUES (uid,'08012345678','MTN',5000,'DATA','VTU-2');
    INSERT INTO crypto_transfers (user_id, chain_type, token, amount, recipient_address, tx_hash) VALUES (uid,'EVM','USDT',50,'0x742d35Cc6634C0532925a3b844Bc454e4438f44e','0xmockhash1');
    INSERT INTO crypto_transfers (user_id, chain_type, token, amount, recipient_address, tx_hash) VALUES (uid,'SOLANA','SOL',1.2,'mockSo11111111111111111111111111111111112','mockSolHash1');
  END IF;
END $$;
