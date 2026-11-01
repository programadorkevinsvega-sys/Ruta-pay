CREATE TABLE catalog_pricing.audit_events (
    event_id UUID PRIMARY KEY,
    actor_id UUID NOT NULL,
    action TEXT NOT NULL,
    resource_id UUID NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('completed','rejected')),
    correlation_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    safe_details JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX catalog_audit_resource_idx ON catalog_pricing.audit_events(resource_id,occurred_at DESC);

CREATE TABLE catalog_pricing.outbox_events (
    event_id UUID PRIMARY KEY,
    aggregate_id UUID NOT NULL,
    aggregate_version BIGINT NOT NULL CHECK (aggregate_version > 0),
    event_type TEXT NOT NULL,
    schema_version SMALLINT NOT NULL DEFAULT 1,
    audience TEXT NOT NULL CHECK (audience IN ('tenant','platform')),
    payload JSONB NOT NULL,
    traceparent TEXT,
    correlation_id TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0,
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    last_error_code TEXT,
    UNIQUE (aggregate_id,aggregate_version)
);
CREATE INDEX catalog_outbox_pending_idx ON catalog_pricing.outbox_events(available_at,occurred_at) WHERE published_at IS NULL;
