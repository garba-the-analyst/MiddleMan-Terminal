-- Migration 0003: Admin enhancements - roles, permissions, analytics, catalogue CRUD

-- 1. Extend admin_employees with more granular permissions
ALTER TABLE admin_employees 
    ADD COLUMN IF NOT EXISTS full_name VARCHAR(128),
    ADD COLUMN IF NOT EXISTS is_active BOOLEAN DEFAULT true,
    ADD COLUMN IF NOT EXISTS last_login TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS permissions JSONB DEFAULT '[]'::jsonb,
    ADD COLUMN IF NOT EXISTS created_by UUID REFERENCES admin_employees(id);

-- 2. Create role_permissions table for granular access control
CREATE TABLE IF NOT EXISTS role_permissions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    role VARCHAR(32) NOT NULL,
    permission VARCHAR(64) NOT NULL,
    description TEXT,
    UNIQUE(role, permission)
);

-- 3. Insert default role permissions
INSERT INTO role_permissions (role, permission, description) VALUES
-- SUPER_ADMIN - full access
('SUPER_ADMIN', 'employees.create', 'Create new employee accounts'),
('SUPER_ADMIN', 'employees.read', 'View employee list and details'),
('SUPER_ADMIN', 'employees.update', 'Update employee details and roles'),
('SUPER_ADMIN', 'employees.delete', 'Deactivate/delete employees'),
('SUPER_ADMIN', 'catalogue.read', 'View price catalogue'),
('SUPER_ADMIN', 'catalogue.create', 'Add new catalogue entries'),
('SUPER_ADMIN', 'catalogue.update', 'Update catalogue rates and status'),
('SUPER_ADMIN', 'catalogue.delete', 'Delete catalogue entries'),
('SUPER_ADMIN', 'trades.read', 'View all trades'),
('SUPER_ADMIN', 'trades.approve', 'Approve trades'),
('SUPER_ADMIN', 'trades.reject', 'Reject trades'),
('SUPER_ADMIN', 'trades.adjust', 'Adjust trade payout amounts'),
('SUPER_ADMIN', 'analytics.read', 'View bot analytics and metrics'),
('SUPER_ADMIN', 'analytics.export', 'Export analytics data'),
('SUPER_ADMIN', 'logs.read', 'View audit logs'),
('SUPER_ADMIN', 'settings.read', 'View system settings'),
('SUPER_ADMIN', 'settings.update', 'Update system settings'),

-- OPERATIONS_MANAGER - operational oversight
('OPERATIONS_MANAGER', 'employees.read', 'View employee list'),
('OPERATIONS_MANAGER', 'employees.stats', 'View employee performance stats'),
('OPERATIONS_MANAGER', 'catalogue.read', 'View price catalogue'),
('OPERATIONS_MANAGER', 'catalogue.update', 'Update catalogue rates'),
('OPERATIONS_MANAGER', 'trades.read', 'View all trades'),
('OPERATIONS_MANAGER', 'trades.approve', 'Approve trades'),
('OPERATIONS_MANAGER', 'trades.reject', 'Reject trades'),
('OPERATIONS_MANAGER', 'trades.adjust', 'Adjust trade payout amounts'),
('OPERATIONS_MANAGER', 'analytics.read', 'View bot analytics and metrics'),
('OPERATIONS_MANAGER', 'analytics.export', 'Export analytics data'),
('OPERATIONS_MANAGER', 'logs.read', 'View audit logs'),

-- COMPLIANCE - compliance and review focus
('COMPLIANCE', 'trades.read', 'View all trades'),
('COMPLIANCE', 'trades.approve', 'Approve trades'),
('COMPLIANCE', 'trades.reject', 'Reject trades'),
('COMPLIANCE', 'trades.adjust', 'Adjust trade payout amounts'),
('COMPLIANCE', 'catalogue.read', 'View price catalogue'),
('COMPLIANCE', 'logs.read', 'View audit logs'),

-- SUPPORT_AGENT - customer support focus
('SUPPORT_AGENT', 'trades.read', 'View trades'),
('SUPPORT_AGENT', 'catalogue.read', 'View price catalogue'),
('SUPPORT_AGENT', 'logs.read', 'View audit logs (own actions only)')

ON CONFLICT (role, permission) DO NOTHING;

-- 4. Add check constraint for new roles
ALTER TABLE admin_employees 
    DROP CONSTRAINT IF EXISTS ck_admin_role,
    ADD CONSTRAINT ck_admin_role CHECK (role IN ('SUPER_ADMIN', 'OPERATIONS_MANAGER', 'COMPLIANCE', 'SUPPORT_AGENT', 'AGENT'));

-- 5. Create bot_analytics table for tracking bot metrics
CREATE TABLE IF NOT EXISTS bot_analytics (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    date DATE NOT NULL DEFAULT CURRENT_DATE,
    metric_name VARCHAR(64) NOT NULL,
    metric_value BIGINT NOT NULL DEFAULT 0,
    metadata JSONB DEFAULT '{}'::jsonb,
    UNIQUE(date, metric_name)
);

-- 6. Create index for analytics queries
CREATE INDEX IF NOT EXISTS idx_bot_analytics_date ON bot_analytics(date DESC);
CREATE INDEX IF NOT EXISTS idx_bot_analytics_metric ON bot_analytics(metric_name);

-- 7. Create price_catalogue_audit table for tracking catalogue changes
CREATE TABLE IF NOT EXISTS price_catalogue_audit (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    catalogue_id BIGINT REFERENCES price_catalogue(id) ON DELETE SET NULL,
    employee_id UUID REFERENCES admin_employees(id) ON DELETE SET NULL,
    action VARCHAR(16) NOT NULL CHECK (action IN ('CREATE', 'UPDATE', 'DELETE')),
    old_values JSONB,
    new_values JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_catalogue_audit_catalogue ON price_catalogue_audit(catalogue_id);
CREATE INDEX IF NOT EXISTS idx_catalogue_audit_employee ON price_catalogue_audit(employee_id);

-- 8. Update price_catalogue to track who made changes
ALTER TABLE price_catalogue 
    ADD COLUMN IF NOT EXISTS created_by UUID REFERENCES admin_employees(id),
    ADD COLUMN IF NOT EXISTS updated_by UUID REFERENCES admin_employees(id);

-- 9. Function to check permission
CREATE OR REPLACE FUNCTION has_permission(emp_id UUID, perm VARCHAR(64))
RETURNS BOOLEAN AS $$
DECLARE
    emp_role VARCHAR(32);
    emp_perms JSONB;
    has_perm BOOLEAN := FALSE;
BEGIN
    SELECT role, permissions INTO emp_role, emp_perms 
    FROM admin_employees WHERE id = emp_id AND is_active = true;
    
    IF emp_role IS NULL THEN
        RETURN FALSE;
    END IF;
    
    -- Check explicit permissions first
    IF emp_perms ? perm THEN
        RETURN TRUE;
    END IF;
    
    -- Check role-based permissions
    SELECT EXISTS(
        SELECT 1 FROM role_permissions 
        WHERE role = emp_role AND permission = perm
    ) INTO has_perm;
    
    RETURN has_perm;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

-- 10. Seed the super_admin (password: Babawo_344, hashed with argon2id)
-- Note: This is a placeholder hash - actual hash will be generated by the application
INSERT INTO admin_employees (email, password_hash, full_name, role, permissions, is_active)
VALUES (
    'garbaabdullahi344@gmail.com',
    '$argon2id$v=19$m=65536,t=3,p=4$PLACEHOLDER_SALT$PLACEHOLDER_HASH',
    'Garba Abdullahi',
    'SUPER_ADMIN',
    '["*"]'::jsonb,
    true
)
ON CONFLICT (email) DO UPDATE SET
    password_hash = EXCLUDED.password_hash,
    full_name = EXCLUDED.full_name,
    role = EXCLUDED.role,
    permissions = EXCLUDED.permissions,
    is_active = EXCLUDED.is_active;