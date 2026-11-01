INSERT INTO identity_access.operator_role_permissions (role, permission) VALUES
    ('platform_owner', 'catalog.manage'),
    ('platform_security', 'catalog.manage')
ON CONFLICT DO NOTHING;

INSERT INTO identity_access.tenant_role_permissions (role, permission) VALUES
    ('tenant_owner', 'usage.read'),
    ('tenant_admin', 'usage.read'),
    ('developer', 'usage.read'),
    ('viewer', 'usage.read'),
    ('tenant_owner', 'usage.consume'),
    ('tenant_admin', 'usage.consume'),
    ('developer', 'usage.consume')
ON CONFLICT DO NOTHING;
