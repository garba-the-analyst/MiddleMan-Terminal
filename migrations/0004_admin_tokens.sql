-- Migration 0004: Admin tokens table for session management

CREATE TABLE IF NOT EXISTS admin_tokens (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    token VARCHAR(64) NOT NULL UNIQUE,
    employee_id UUID NOT NULL REFERENCES admin_employees(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_admin_tokens_token ON admin_tokens(token);
CREATE INDEX IF NOT EXISTS idx_admin_tokens_employee ON admin_tokens(employee_id);
CREATE INDEX IF NOT EXISTS idx_admin_tokens_expires ON admin_tokens(expires_at);