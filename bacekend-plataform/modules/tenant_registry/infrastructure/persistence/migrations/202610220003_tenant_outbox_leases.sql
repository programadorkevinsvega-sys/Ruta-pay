ALTER TABLE tenant_registry.outbox_events
    ADD COLUMN available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN lease_token UUID,
    ADD COLUMN lease_until TIMESTAMPTZ,
    ADD COLUMN last_error_code TEXT,
    ADD COLUMN correlation_id TEXT NOT NULL DEFAULT 'migration';

CREATE INDEX tenant_registry_outbox_available_idx
    ON tenant_registry.outbox_events(available_at, occurred_at)
    WHERE published_at IS NULL;

ALTER TABLE tenant_registry.outbox_events
    ADD CONSTRAINT tenant_registry_outbox_lease_pair
    CHECK ((lease_token IS NULL) = (lease_until IS NULL));
