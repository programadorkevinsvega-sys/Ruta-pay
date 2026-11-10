ALTER TABLE payments.audit_events
    ALTER COLUMN actor_key_id DROP NOT NULL;

ALTER TABLE payments.audit_events
    ADD COLUMN actor_service_identity TEXT
        CHECK (
            actor_service_identity IS NULL OR
            (char_length(actor_service_identity) BETWEEN 1 AND 128
                AND actor_service_identity ~ '^[a-z][a-z0-9._:-]*$')
        ),
    ADD CONSTRAINT payment_audit_actor_identity_check
        CHECK ((actor_key_id IS NOT NULL) <> (actor_service_identity IS NOT NULL));

CREATE INDEX payment_audit_service_actor_idx
    ON payments.audit_events(tenant_id, actor_service_identity, occurred_at DESC)
    WHERE actor_service_identity IS NOT NULL;
