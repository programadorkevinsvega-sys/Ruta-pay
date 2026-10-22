ALTER TABLE identity_access.outbox_events
    ADD COLUMN available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN lease_token UUID,
    ADD COLUMN lease_until TIMESTAMPTZ,
    ADD COLUMN last_error_code TEXT;

ALTER TABLE identity_access.outbox_events
    ADD COLUMN correlation_id TEXT NOT NULL DEFAULT 'migration';

CREATE TABLE identity_access.consumer_inbox (
    consumer_name TEXT NOT NULL,
    event_id UUID NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    correlation_id TEXT NOT NULL,
    PRIMARY KEY (consumer_name, event_id)
);

CREATE INDEX identity_outbox_available_idx
    ON identity_access.outbox_events(available_at, occurred_at)
    WHERE published_at IS NULL;

ALTER TABLE identity_access.outbox_events
    ADD CONSTRAINT identity_outbox_lease_pair
    CHECK ((lease_token IS NULL) = (lease_until IS NULL));
