-- 0007: auto rates (non-giftcard) + platform fees (updatable via dashboard)

-- platform_fees: one row per tx_type, fee = fixed + percent, gas is separate network fee
CREATE TABLE IF NOT EXISTS platform_fees (
  id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
  fee_type VARCHAR(32) NOT NULL UNIQUE CHECK (fee_type IN (
    'FIAT_PAYOUT','P2P_TRANSFER','AIRTIME','DATA','UTILITY',
    'CRYPTO_TRANSFER','OFFRAMP','ONRAMP',
    'SPOT','FUTURES','DEGEN',
    'GIFT_CARD' -- kept 0, giftcard uses price_catalogue manual rate
  )),
  fixed_amount NUMERIC(18,2) NOT NULL DEFAULT 0,
  percent NUMERIC(5,2) NOT NULL DEFAULT 0, -- e.g. 1.00 = 1%
  currency VARCHAR(8) NOT NULL DEFAULT 'NGN',
  is_active BOOLEAN NOT NULL DEFAULT true,
  updated_by UUID REFERENCES admin_employees(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TRIGGER trg_platform_fees_updated BEFORE UPDATE ON platform_fees FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Seed default fees (admin can edit later; GIFT_CARD 0)
INSERT INTO platform_fees (fee_type, fixed_amount, percent, currency) VALUES
('FIAT_PAYOUT', 50.00, 0.50, 'NGN'),
('P2P_TRANSFER', 0.00, 0.00, 'NGN'),
('AIRTIME', 0.00, 0.00, 'NGN'),
('DATA', 0.00, 0.00, 'NGN'),
('UTILITY', 0.00, 1.00, 'NGN'),
('CRYPTO_TRANSFER', 0.00, 0.50, 'NGN'), -- + gas outside
('OFFRAMP', 0.00, 1.20, 'NGN'),
('ONRAMP', 0.00, 1.00, 'NGN'),
('SPOT', 0.00, 0.80, 'NGN'),
('FUTURES', 0.00, 1.00, 'NGN'),
('DEGEN', 0.00, 1.50, 'NGN'),
('GIFT_CARD', 0.00, 0.00, 'NGN')
ON CONFLICT (fee_type) DO NOTHING;

-- rate_sources: which pairs are auto-updated, interval, last status
CREATE TABLE IF NOT EXISTS rate_sources (
  pair VARCHAR(16) PRIMARY KEY, -- e.g. USD/NGN, SOL/NGN, BTC/NGN
  source VARCHAR(32) NOT NULL, -- coingecko, exchangerate-api, frankfurter
  auto_update BOOLEAN NOT NULL DEFAULT true,
  interval_seconds INT NOT NULL DEFAULT 300, -- 5m crypto, 3600 fiat
  last_fetched_at TIMESTAMPTZ,
  last_rate NUMERIC(20,8),
  last_error TEXT,
  is_giftcard BOOLEAN NOT NULL DEFAULT false
);
INSERT INTO rate_sources (pair, source, interval_seconds, auto_update, is_giftcard) VALUES
('USD/NGN','exchangerate-api',3600,true,false),
('GBP/NGN','exchangerate-api',3600,true,false),
('EUR/NGN','exchangerate-api',3600,true,false),
('USDT/NGN','coingecko',300,true,false),
('SOL/NGN','coingecko',300,true,false),
('ETH/NGN','coingecko',300,true,false),
('BTC/NGN','coingecko',300,true,false),
('BNB/NGN','coingecko',300,true,false)
ON CONFLICT (pair) DO UPDATE SET auto_update=EXCLUDED.auto_update;

-- Ensure giftcard pairs are NOT auto-updated (manual via price_catalogue)
-- price_catalogue stays manual; rate_sources is_giftcard=false ensures separation

-- Helper to calc fee
CREATE OR REPLACE FUNCTION calc_platform_fee(p_fee_type VARCHAR, p_amount NUMERIC)
RETURNS NUMERIC AS $$
DECLARE r RECORD; fee NUMERIC;
BEGIN
  SELECT fixed_amount, percent INTO r FROM platform_fees WHERE fee_type=p_fee_type AND is_active=true;
  IF NOT FOUND THEN RETURN 0; END IF;
  fee := r.fixed_amount + (p_amount * r.percent / 100.0);
  RETURN ROUND(fee, 2);
END; $$ LANGUAGE plpgsql;
