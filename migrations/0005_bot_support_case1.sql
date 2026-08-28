-- Migration 0005: Case Study 1 - AI Customer Support Assistant
-- Bot interactions, knowledge base, enriched analytics

-- 1. Bot interactions log (core for Case Study 1 requirements)
CREATE TABLE IF NOT EXISTS bot_interactions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    message_id VARCHAR(128) UNIQUE,
    whatsapp_number VARCHAR(32) NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    inbound_text TEXT NOT NULL,
    intent VARCHAR(64) NOT NULL DEFAULT 'UNKNOWN',
    category VARCHAR(32) NOT NULL DEFAULT 'general', -- delivery, payment, refund, complaint, product_enquiry, gift_card, balance, p2p, contract, general, faq
    sentiment VARCHAR(16) NOT NULL DEFAULT 'neutral' CHECK (sentiment IN ('positive','neutral','negative')),
    urgency VARCHAR(16) NOT NULL DEFAULT 'low' CHECK (urgency IN ('low','medium','high','critical')),
    urgency_score INT NOT NULL DEFAULT 0, -- 0-100
    confidence NUMERIC(3,2) DEFAULT 0.5,
    response_text TEXT,
    kb_article_id UUID,
    escalated BOOLEAN DEFAULT false,
    escalation_reason TEXT,
    assigned_agent UUID REFERENCES admin_employees(id) ON DELETE SET NULL,
    resolved BOOLEAN DEFAULT false,
    handling_ms INT DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_bot_inter_category ON bot_interactions(category);
CREATE INDEX IF NOT EXISTS idx_bot_inter_sentiment ON bot_interactions(sentiment);
CREATE INDEX IF NOT EXISTS idx_bot_inter_urgency ON bot_interactions(urgency);
CREATE INDEX IF NOT EXISTS idx_bot_inter_escalated ON bot_interactions(escalated) WHERE escalated = true;
CREATE INDEX IF NOT EXISTS idx_bot_inter_created ON bot_interactions(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_bot_inter_intent ON bot_interactions(intent);

-- 2. Knowledge base (FAQs, policies, product info)
CREATE TABLE IF NOT EXISTS knowledge_base (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    category VARCHAR(32) NOT NULL, -- delivery, payment, refund, complaint, product_enquiry, gift_card, wallet, security
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    keywords TEXT[] DEFAULT '{}',
    source VARCHAR(64) DEFAULT 'internal',
    priority INT DEFAULT 0,
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_kb_category ON knowledge_base(category);
CREATE INDEX IF NOT EXISTS idx_kb_active ON knowledge_base(is_active) WHERE is_active = true;

-- 3. Seed knowledge base (10 realistic entries)
INSERT INTO knowledge_base (category, question, answer, keywords, priority) VALUES
('gift_card', 'What gift cards do you accept and at what rates?', 'We accept STEAM ($1450/$), APPLE ($1500/$), AMAZON ($1420/$), RAZER GOLD ($1380/$), GOOGLE PLAY ($1360/$). Rates update daily. Send a clear photo of the card.', ARRAY['gift','card','rate','steam','apple'], 10),
('gift_card', 'How long does gift card liquidation take?', 'Most cards are verified within 15-45 minutes. PHYSICAL cards may take up to 2 hours. You will be notified on WhatsApp.', ARRAY['how long','time','liquidation','verify'], 9),
('payment', 'My payment failed but I was debited', 'If debited but not credited, it auto-reverses within 24h. Share your transaction ID and we will trace it. For urgent cases we escalate to COMPLIANCE.', ARRAY['payment','failed','debited','charged'], 10),
('refund', 'How do I get a refund for a rejected card?', 'Rejected cards are not charged. If you were debited, open a dispute with reason and our support will refund within 24 hours.', ARRAY['refund','rejected','returned'], 8),
('delivery', 'When will my wallet be credited after approval?', 'Instantly. Once an agent approves, your NGN wallet is credited and you get a WhatsApp confirmation. You can request withdrawal to your bank.', ARRAY['wallet','credited','when','delivery'], 9),
('wallet', 'What is the minimum and maximum card value?', 'We accept $10 - $2000 per card. Below $10 we cannot process. Above $2000 please split into multiple cards.', ARRAY['minimum','maximum','limit'], 7),
('security', 'Is my card code safe?', 'Yes. Codes are encrypted with AES-256-GCM and auto-deleted after verification. We never share codes outside the verification flow.', ARRAY['safe','secure','code','privacy'], 8),
('product_enquiry', 'Do you buy ECODE or PHYSICAL?', 'Both. ECODE is an online code, PHYSICAL is a plastic card photo. ECODE settles faster (lower fraud check).', ARRAY['ecode','physical','difference'], 6),
('complaint', 'My trade has been pending too long', 'We understand. High-risk brands take longer. If >2 hours, reply URGENT and we will escalate to Operations Manager.', ARRAY['pending','too long','delay','complaint'], 9),
('payment', 'Can I send money to another user?', 'Yes: Send "Send 5000 to 08012345678". The recipient must be on MiddleMan. Fee is ₦0 for now.', ARRAY['send','transfer','p2p'], 7)
ON CONFLICT DO NOTHING;

-- 4. Seed bot_analytics with 14 days of realistic metrics (for dashboard charts)
INSERT INTO bot_analytics (date, metric_name, metric_value, metadata) VALUES
(CURRENT_DATE - 13, 'messages_inbound', 142, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 12, 'messages_inbound', 168, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 11, 'messages_inbound', 155, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 10, 'messages_inbound', 189, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 9, 'messages_inbound', 210, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 8, 'messages_inbound', 198, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 7, 'messages_inbound', 225, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 6, 'messages_inbound', 240, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 5, 'messages_inbound', 198, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 4, 'messages_inbound', 265, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 3, 'messages_inbound', 278, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 2, 'messages_inbound', 312, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 1, 'messages_inbound', 298, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE, 'messages_inbound', 45, '{"channel":"whatsapp"}'::jsonb),
(CURRENT_DATE - 7, 'escalated', 12, '{}'::jsonb),
(CURRENT_DATE - 6, 'escalated', 9, '{}'::jsonb),
(CURRENT_DATE - 5, 'escalated', 14, '{}'::jsonb),
(CURRENT_DATE - 4, 'escalated', 18, '{}'::jsonb),
(CURRENT_DATE - 3, 'escalated', 11, '{}'::jsonb),
(CURRENT_DATE - 2, 'escalated', 22, '{}'::jsonb),
(CURRENT_DATE - 1, 'escalated', 16, '{}'::jsonb),
(CURRENT_DATE, 'escalated', 3, '{}'::jsonb),
(CURRENT_DATE, 'auto_resolved', 38, '{}'::jsonb),
(CURRENT_DATE - 1, 'auto_resolved', 267, '{}'::jsonb),
(CURRENT_DATE, 'avg_handling_ms', 1840, '{}'::jsonb),
(CURRENT_DATE, 'knowledge_base_hits', 28, '{}'::jsonb)
ON CONFLICT (date, metric_name) DO UPDATE SET metric_value = EXCLUDED.metric_value;

-- 5. Seed bot_interactions (120 rows over 14 days, realistic distribution for demo)
DO $$
DECLARE
  cats TEXT[] := ARRAY['gift_card','payment','refund','complaint','product_enquiry','balance','p2p','contract','general','faq'];
  sents TEXT[] := ARRAY['positive','neutral','negative'];
  urges TEXT[] := ARRAY['low','medium','high','critical'];
  ints TEXT[] := ARRAY['LIQUIDATE_GIFT_CARD','CHECK_BALANCE','P2P_TRANSFER','CHECK_CONTRACT_SECURITY','HELP','UNKNOWN','FAQ'];
  i INT;
  c TEXT; s TEXT; u TEXT; it TEXT; esc BOOL; days_ago INT;
BEGIN
  FOR i IN 1..120 LOOP
    c := cats[1 + floor(random()*array_length(cats,1))::int];
    s := CASE WHEN random() < 0.15 THEN 'negative' WHEN random() < 0.45 THEN 'positive' ELSE 'neutral' END;
    u := CASE WHEN s='negative' AND random() < 0.6 THEN (ARRAY['high','critical'])[1+floor(random()*2)::int] WHEN random() < 0.12 THEN 'high' ELSE (ARRAY['low','medium'])[1+floor(random()*2)::int] END;
    it := ints[1 + floor(random()*array_length(ints,1))::int];
    esc := (u IN ('high','critical') AND s='negative') OR (random() < 0.08);
    days_ago := floor(random()*14)::int;
    INSERT INTO bot_interactions (whatsapp_number, inbound_text, intent, category, sentiment, urgency, urgency_score, confidence, response_text, escalated, escalation_reason, handling_ms, created_at)
    VALUES (
      '23480' || lpad((700000000 + floor(random()*99999999)::int)::text, 9, '0'),
      CASE c WHEN 'gift_card' THEN 'I wan sell $' || (10+floor(random()*190)::int)::text || ' ' || (ARRAY['Steam','Apple','Amazon','Razer'])[1+floor(random()*4)::int] || ' card'
             WHEN 'complaint' THEN (ARRAY['My card still pending since morning, this is urgent!','You people are delaying my money, I need refund now','Why is my trade rejected?'])[1+floor(random()*3)::int]
             WHEN 'payment' THEN 'My payment failed but I was debited ₦' || (1000+floor(random()*50000)::int)::text
             WHEN 'refund' THEN 'I need refund for rejected card'
             ELSE 'Hello, ' || c || ' enquiry #' || i::text END,
      it, c, s, u,
      CASE u WHEN 'low' THEN 20+floor(random()*20)::int WHEN 'medium' THEN 40+floor(random()*20)::int WHEN 'high' THEN 70+floor(random()*15)::int ELSE 90+floor(random()*10)::int END,
      0.55 + random()*0.40,
      'Auto-response for ' || c,
      esc,
      CASE WHEN esc THEN (ARRAY['Negative sentiment + high urgency','Low confidence','Sensitive complaint keyword'])[1+floor(random()*3)::int] ELSE NULL END,
      800 + floor(random()*4000)::int,
      NOW() - (days_ago || ' days')::interval - (floor(random()*1440) || ' minutes')::interval
    );
  END LOOP;
END $$;
