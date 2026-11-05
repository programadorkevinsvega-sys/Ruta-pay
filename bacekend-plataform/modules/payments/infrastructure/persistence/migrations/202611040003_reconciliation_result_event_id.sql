ALTER TABLE payments.payment_reconciliation_attempts
    ADD COLUMN result_event_id UUID;

CREATE UNIQUE INDEX payment_reconciliation_result_event_unique
    ON payments.payment_reconciliation_attempts (
        tenant_id,
        environment,
        result_event_id
    )
    WHERE result_event_id IS NOT NULL;
