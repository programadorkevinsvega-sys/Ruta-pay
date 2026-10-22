CREATE SCHEMA IF NOT EXISTS tenant_registry;

CREATE TABLE tenant_registry.tenants (
    id UUID PRIMARY KEY,
    display_name TEXT NOT NULL CHECK (char_length(display_name) BETWEEN 2 AND 120),
    slug TEXT NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$'),
    status TEXT NOT NULL CHECK (status IN ('pending_verification', 'active', 'suspended', 'closing', 'closed')),
    created_by UUID NOT NULL,
    verification_reason TEXT,
    status_version BIGINT NOT NULL DEFAULT 1 CHECK (status_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at TIMESTAMPTZ,
    UNIQUE (id, status)
);

CREATE TABLE tenant_registry.status_history (
    tenant_id UUID NOT NULL REFERENCES tenant_registry.tenants(id),
    version BIGINT NOT NULL CHECK (version > 0),
    previous_status TEXT CHECK (previous_status IN ('pending_verification', 'active', 'suspended', 'closing', 'closed')),
    new_status TEXT NOT NULL CHECK (new_status IN ('pending_verification', 'active', 'suspended', 'closing', 'closed')),
    actor_id UUID NOT NULL,
    reason TEXT NOT NULL CHECK (char_length(reason) BETWEEN 3 AND 500),
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, version)
);

ALTER TABLE tenant_registry.tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_registry.tenants FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_registry_scope
    ON tenant_registry.tenants
    USING (
        id = nullif(current_setting('app.tenant_id', true), '')::uuid
        OR current_setting('app.operator_scope', true) = 'true'
    )
    WITH CHECK (current_setting('app.operator_scope', true) = 'true');

CREATE POLICY tenant_registry_registration_insert
    ON tenant_registry.tenants
    FOR INSERT
    WITH CHECK (
        status = 'pending_verification'
        AND created_by = nullif(current_setting('app.actor_id', true), '')::uuid
    );

ALTER TABLE tenant_registry.status_history ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_registry.status_history FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_registry_history_scope
    ON tenant_registry.status_history
    USING (
        tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid
        OR current_setting('app.operator_scope', true) = 'true'
    )
    WITH CHECK (current_setting('app.operator_scope', true) = 'true');

CREATE POLICY tenant_registry_registration_history_insert
    ON tenant_registry.status_history
    FOR INSERT
    WITH CHECK (
        actor_id = nullif(current_setting('app.actor_id', true), '')::uuid
        AND new_status = 'pending_verification'
        AND previous_status IS NULL
    );

CREATE TABLE tenant_registry.outbox_events (
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

CREATE INDEX tenant_registry_outbox_pending_idx
    ON tenant_registry.outbox_events(occurred_at)
    WHERE published_at IS NULL;
