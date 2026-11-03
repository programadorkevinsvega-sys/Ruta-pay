CREATE SCHEMA IF NOT EXISTS payments;

CREATE TABLE payments.payment_intents (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test','live')),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    payment_method_type TEXT NOT NULL CHECK (payment_method_type IN ('card','pse','crypto')),
    merchant_reference TEXT CHECK (merchant_reference IS NULL OR char_length(merchant_reference) BETWEEN 1 AND 128),
    status TEXT NOT NULL CHECK (status IN ('created')),
    aggregate_version BIGINT NOT NULL CHECK (aggregate_version = 1),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,environment,id)
);

CREATE INDEX payment_intents_tenant_created_idx
    ON payments.payment_intents(tenant_id,created_at DESC,id);

ALTER TABLE payments.payment_intents ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.payment_intents FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_intents_tenant_isolation ON payments.payment_intents
    USING (tenant_id = nullif(current_setting('app.tenant_id',true),'')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id',true),'')::uuid);

CREATE TABLE payments.idempotency_records (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test','live')),
    operation TEXT NOT NULL CHECK (operation = 'payment_intent.create.v1'),
    idempotency_key TEXT NOT NULL CHECK (char_length(idempotency_key) BETWEEN 1 AND 128),
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash) = 32),
    payment_intent_id UUID,
    response_snapshot JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,environment,operation,idempotency_key),
    FOREIGN KEY (tenant_id,environment,payment_intent_id)
        REFERENCES payments.payment_intents(tenant_id,environment,id)
);

ALTER TABLE payments.idempotency_records ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.idempotency_records FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_idempotency_tenant_isolation ON payments.idempotency_records
    USING (tenant_id = nullif(current_setting('app.tenant_id',true),'')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id',true),'')::uuid);

CREATE TABLE payments.audit_events (
    event_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    actor_key_id UUID NOT NULL,
    action TEXT NOT NULL,
    resource_id UUID NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('completed','denied','failed')),
    correlation_id TEXT NOT NULL,
    trace_id TEXT,
    safe_details JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX payment_audit_tenant_time_idx
    ON payments.audit_events(tenant_id,occurred_at DESC);

ALTER TABLE payments.audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_audit_tenant_isolation ON payments.audit_events
    USING (tenant_id = nullif(current_setting('app.tenant_id',true),'')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id',true),'')::uuid);

CREATE TABLE payments.outbox_events (
    event_id UUID PRIMARY KEY,
    aggregate_id UUID NOT NULL,
    aggregate_version BIGINT NOT NULL CHECK (aggregate_version > 0),
    event_type TEXT NOT NULL,
    schema_version SMALLINT NOT NULL CHECK (schema_version > 0),
    audience TEXT NOT NULL CHECK (audience IN ('tenant','platform')),
    payload JSONB NOT NULL,
    traceparent TEXT,
    correlation_id TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    last_error_code TEXT,
    UNIQUE (aggregate_id,aggregate_version)
);

CREATE INDEX payment_outbox_pending_idx
    ON payments.outbox_events(available_at,occurred_at)
    WHERE published_at IS NULL;
