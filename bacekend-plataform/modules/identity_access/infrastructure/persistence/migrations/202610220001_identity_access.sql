CREATE SCHEMA IF NOT EXISTS identity_access;

CREATE TABLE identity_access.tenant_users (
    id UUID PRIMARY KEY,
    email_normalized TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'locked', 'closed')),
    email_verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE identity_access.operator_users (
    id UUID PRIMARY KEY,
    email_normalized TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'locked', 'closed')),
    role TEXT NOT NULL CHECK (role IN ('platform_owner', 'platform_security', 'platform_support')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE identity_access.tenant_memberships (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES identity_access.tenant_users(id),
    role TEXT NOT NULL CHECK (role IN ('tenant_owner', 'tenant_admin', 'developer', 'viewer')),
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended', 'revoked')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, user_id)
);

CREATE TABLE identity_access.tenant_role_permissions (
    role TEXT NOT NULL CHECK (role IN ('tenant_owner', 'tenant_admin', 'developer', 'viewer')),
    permission TEXT NOT NULL,
    PRIMARY KEY (role, permission)
);

INSERT INTO identity_access.tenant_role_permissions (role, permission) VALUES
    ('tenant_owner', 'tenant.read'),
    ('tenant_owner', 'members.manage'),
    ('tenant_owner', 'api_keys.manage'),
    ('tenant_owner', 'tenant.settings.manage'),
    ('tenant_admin', 'tenant.read'),
    ('tenant_admin', 'members.manage'),
    ('tenant_admin', 'api_keys.manage'),
    ('tenant_admin', 'tenant.settings.manage'),
    ('developer', 'tenant.read'),
    ('developer', 'api_keys.manage'),
    ('viewer', 'tenant.read');

CREATE TABLE identity_access.operator_role_permissions (
    role TEXT NOT NULL CHECK (role IN ('platform_owner', 'platform_security', 'platform_support')),
    permission TEXT NOT NULL,
    PRIMARY KEY (role, permission)
);

INSERT INTO identity_access.operator_role_permissions (role, permission) VALUES
    ('platform_owner', 'tenants.read'),
    ('platform_owner', 'tenants.verify'),
    ('platform_owner', 'tenants.activate'),
    ('platform_owner', 'tenants.suspend'),
    ('platform_owner', 'tenants.close'),
    ('platform_owner', 'operators.bootstrap'),
    ('platform_security', 'tenants.read'),
    ('platform_security', 'tenants.verify'),
    ('platform_security', 'tenants.activate'),
    ('platform_security', 'tenants.suspend'),
    ('platform_security', 'tenants.close'),
    ('platform_support', 'tenants.read');

CREATE TABLE identity_access.sessions (
    id UUID PRIMARY KEY,
    subject_kind TEXT NOT NULL CHECK (subject_kind IN ('tenant_user', 'operator_user')),
    subject_id UUID NOT NULL,
    token_digest BYTEA NOT NULL UNIQUE CHECK (octet_length(token_digest) = 32),
    csrf_digest BYTEA NOT NULL CHECK (octet_length(csrf_digest) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (expires_at > created_at)
);

CREATE INDEX sessions_subject_active_idx
    ON identity_access.sessions(subject_kind, subject_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE identity_access.api_keys (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    name TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
    prefix TEXT NOT NULL,
    secret_digest BYTEA NOT NULL UNIQUE CHECK (octet_length(secret_digest) = 32),
    scopes TEXT[] NOT NULL CHECK (cardinality(scopes) > 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
    created_by UUID NOT NULL REFERENCES identity_access.tenant_users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, environment, name)
);

ALTER TABLE identity_access.tenant_memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE identity_access.tenant_memberships FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_memberships_tenant_isolation
    ON identity_access.tenant_memberships
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);

ALTER TABLE identity_access.api_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE identity_access.api_keys FORCE ROW LEVEL SECURITY;
CREATE POLICY api_keys_tenant_isolation
    ON identity_access.api_keys
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);

CREATE TABLE identity_access.outbox_events (
    event_id UUID PRIMARY KEY,
    aggregate_id UUID NOT NULL,
    aggregate_version BIGINT NOT NULL CHECK (aggregate_version > 0),
    event_type TEXT NOT NULL,
    schema_version SMALLINT NOT NULL CHECK (schema_version > 0),
    audience TEXT NOT NULL,
    payload JSONB NOT NULL,
    traceparent TEXT,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    UNIQUE (aggregate_id, aggregate_version)
);

CREATE INDEX identity_outbox_pending_idx
    ON identity_access.outbox_events(occurred_at)
    WHERE published_at IS NULL;

CREATE TABLE identity_access.audit_events (
    event_id UUID PRIMARY KEY,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('tenant_user', 'operator_user', 'system')),
    actor_id UUID,
    tenant_id UUID,
    action TEXT NOT NULL,
    resource_id UUID,
    outcome TEXT NOT NULL CHECK (outcome IN ('allowed', 'denied', 'completed', 'failed')),
    correlation_id TEXT NOT NULL,
    trace_id TEXT,
    safe_details JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX identity_audit_tenant_time_idx
    ON identity_access.audit_events(tenant_id, occurred_at DESC);
