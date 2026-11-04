ALTER TABLE payments.payment_intents
    ADD CONSTRAINT payment_intents_scope_money_unique
    UNIQUE (tenant_id, environment, id, amount_minor, currency);

CREATE TABLE payments.payment_attempts (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
    payment_intent_id UUID NOT NULL,
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    provider_code TEXT NOT NULL CHECK (provider_code ~ '^[a-z][a-z0-9_]{1,31}$'),
    provider_account_reference TEXT NOT NULL
        CHECK (char_length(provider_account_reference) BETWEEN 1 AND 128)
        CHECK (provider_account_reference !~ '[[:cntrl:]]'),
    status TEXT NOT NULL CHECK (
        status IN (
            'created', 'submitted', 'processing', 'requires_action',
            'authorized', 'succeeded', 'failed', 'outcome_unknown'
        )
    ),
    aggregate_version BIGINT NOT NULL DEFAULT 1 CHECK (aggregate_version > 0),
    last_event_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, environment, id),
    UNIQUE (tenant_id, environment, payment_intent_id, attempt_number),
    UNIQUE (tenant_id, environment, id, payment_intent_id),
    UNIQUE (
        tenant_id, environment, id, payment_intent_id,
        provider_code, provider_account_reference
    ),
    FOREIGN KEY (tenant_id, environment, payment_intent_id)
        REFERENCES payments.payment_intents (tenant_id, environment, id)
);

CREATE INDEX payment_attempts_intent_history_idx
    ON payments.payment_attempts (tenant_id, environment, payment_intent_id, attempt_number DESC);

CREATE INDEX payment_attempts_reconciliation_idx
    ON payments.payment_attempts (tenant_id, environment, status, updated_at)
    WHERE status = 'outcome_unknown';

ALTER TABLE payments.payment_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.payment_attempts FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_attempts_tenant_isolation ON payments.payment_attempts
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);

CREATE TABLE payments.payment_transactions (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
    payment_intent_id UUID NOT NULL,
    payment_attempt_id UUID NOT NULL,
    provider_code TEXT NOT NULL CHECK (provider_code ~ '^[a-z][a-z0-9_]{1,31}$'),
    provider_account_reference TEXT NOT NULL
        CHECK (char_length(provider_account_reference) BETWEEN 1 AND 128)
        CHECK (provider_account_reference !~ '[[:cntrl:]]'),
    provider_transaction_reference TEXT NOT NULL
        CHECK (char_length(provider_transaction_reference) BETWEEN 1 AND 160)
        CHECK (provider_transaction_reference !~ '[[:cntrl:]]'),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'authorized', 'succeeded', 'failed', 'outcome_unknown')
    ),
    aggregate_version BIGINT NOT NULL DEFAULT 1 CHECK (aggregate_version > 0),
    last_event_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, environment, id),
    UNIQUE (
        tenant_id, environment, id, payment_intent_id, payment_attempt_id
    ),
    UNIQUE (provider_code, provider_account_reference, provider_transaction_reference),
    FOREIGN KEY (
        tenant_id, environment, payment_intent_id, amount_minor, currency
    ) REFERENCES payments.payment_intents (
        tenant_id, environment, id, amount_minor, currency
    ),
    FOREIGN KEY (
        tenant_id, environment, payment_attempt_id, payment_intent_id,
        provider_code, provider_account_reference
    ) REFERENCES payments.payment_attempts (
        tenant_id, environment, id, payment_intent_id,
        provider_code, provider_account_reference
    )
);

CREATE INDEX payment_transactions_intent_history_idx
    ON payments.payment_transactions (
        tenant_id, environment, payment_intent_id, created_at DESC, id
    );
CREATE INDEX payment_transactions_attempt_idx
    ON payments.payment_transactions (
        tenant_id, environment, payment_attempt_id, created_at, id
    );

CREATE FUNCTION payments.require_submitted_attempt_for_transaction()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    attempt_status TEXT;
BEGIN
    SELECT status
      INTO attempt_status
      FROM payments.payment_attempts
     WHERE tenant_id = NEW.tenant_id
       AND environment = NEW.environment
       AND id = NEW.payment_attempt_id
       AND payment_intent_id = NEW.payment_intent_id
       AND provider_code = NEW.provider_code
       AND provider_account_reference = NEW.provider_account_reference
     FOR KEY SHARE;

    IF attempt_status IS NULL THEN
        RAISE EXCEPTION 'payment attempt relationship is invalid'
            USING ERRCODE = '23503';
    END IF;

    IF attempt_status = 'created' THEN
        RAISE EXCEPTION 'payment attempt must be submitted before a transaction exists'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER payment_transaction_requires_submitted_attempt
    BEFORE INSERT OR UPDATE OF
        tenant_id, environment, payment_intent_id, payment_attempt_id,
        provider_code, provider_account_reference
    ON payments.payment_transactions
    FOR EACH ROW
    EXECUTE FUNCTION payments.require_submitted_attempt_for_transaction();

ALTER TABLE payments.payment_transactions ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.payment_transactions FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_transactions_tenant_isolation ON payments.payment_transactions
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);

CREATE TABLE payments.provider_event_inbox (
    inbox_id UUID PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
    payment_intent_id UUID NOT NULL,
    payment_attempt_id UUID NOT NULL,
    provider_code TEXT NOT NULL CHECK (provider_code ~ '^[a-z][a-z0-9_]{1,31}$'),
    provider_account_reference TEXT NOT NULL
        CHECK (char_length(provider_account_reference) BETWEEN 1 AND 128)
        CHECK (provider_account_reference !~ '[[:cntrl:]]'),
    provider_event_id TEXT NOT NULL
        CHECK (char_length(provider_event_id) BETWEEN 1 AND 160)
        CHECK (provider_event_id !~ '[[:cntrl:]]'),
    provider_transaction_reference TEXT
        CHECK (
            provider_transaction_reference IS NULL OR
            (char_length(provider_transaction_reference) BETWEEN 1 AND 160
                AND provider_transaction_reference !~ '[[:cntrl:]]')
        ),
    normalized_status TEXT NOT NULL CHECK (
        normalized_status IN ('processing', 'requires_action', 'authorized', 'succeeded', 'failed')
    ),
    schema_version SMALLINT NOT NULL CHECK (schema_version = 1),
    occurred_at TIMESTAMPTZ NOT NULL,
    payload_sha256 BYTEA NOT NULL CHECK (octet_length(payload_sha256) = 32),
    correlation_id TEXT NOT NULL CHECK (char_length(correlation_id) BETWEEN 1 AND 128),
    producer_identity TEXT NOT NULL CHECK (char_length(producer_identity) BETWEEN 1 AND 256),
    processing_status TEXT NOT NULL DEFAULT 'received' CHECK (
        processing_status IN ('received', 'applied', 'stale', 'reconciliation_required')
    ),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    UNIQUE (tenant_id, environment, inbox_id),
    UNIQUE (
        provider_code, provider_account_reference, provider_event_id
    ),
    FOREIGN KEY (
        tenant_id, environment, payment_attempt_id, payment_intent_id,
        provider_code, provider_account_reference
    ) REFERENCES payments.payment_attempts (
        tenant_id, environment, id, payment_intent_id,
        provider_code, provider_account_reference
    )
);

CREATE INDEX payment_event_inbox_attempt_time_idx
    ON payments.provider_event_inbox (
        tenant_id, environment, payment_attempt_id, occurred_at DESC
    );

ALTER TABLE payments.provider_event_inbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.provider_event_inbox FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_event_inbox_tenant_isolation ON payments.provider_event_inbox
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);

CREATE TABLE payments.provider_event_deliveries (
    delivery_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
    inbox_id UUID NOT NULL,
    event_id UUID NOT NULL,
    producer_identity TEXT NOT NULL CHECK (char_length(producer_identity) BETWEEN 1 AND 256),
    disposition TEXT NOT NULL CHECK (disposition IN ('accepted', 'duplicate', 'rejected')),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (tenant_id, environment, inbox_id)
        REFERENCES payments.provider_event_inbox (tenant_id, environment, inbox_id)
);

CREATE INDEX payment_event_deliveries_inbox_time_idx
    ON payments.provider_event_deliveries (tenant_id, environment, inbox_id, received_at DESC);

ALTER TABLE payments.provider_event_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.provider_event_deliveries FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_event_deliveries_tenant_isolation ON payments.provider_event_deliveries
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);

CREATE TABLE payments.payment_reconciliation_cases (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
    payment_intent_id UUID NOT NULL,
    payment_attempt_id UUID NOT NULL,
    payment_transaction_id UUID,
    reason TEXT NOT NULL CHECK (
        reason IN ('outcome_unknown', 'late_event', 'terminal_conflict')
    ),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (
        status IN ('pending', 'leased', 'resolved', 'manual_review')
    ),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    last_error_code TEXT,
    result_status TEXT CHECK (
        result_status IS NULL OR
        result_status IN ('processing', 'requires_action', 'authorized', 'succeeded', 'failed')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    UNIQUE (tenant_id, environment, id),
    FOREIGN KEY (tenant_id, environment, payment_attempt_id, payment_intent_id)
        REFERENCES payments.payment_attempts (
            tenant_id, environment, id, payment_intent_id
        ),
    FOREIGN KEY (
        tenant_id, environment, payment_transaction_id, payment_intent_id, payment_attempt_id
    ) REFERENCES payments.payment_transactions (
        tenant_id, environment, id, payment_intent_id, payment_attempt_id
    ),
    CHECK ((lease_token IS NULL) = (lease_until IS NULL)),
    CHECK ((status = 'leased') = (lease_token IS NOT NULL)),
    CHECK ((status = 'resolved') = (resolved_at IS NOT NULL))
);

CREATE UNIQUE INDEX payment_reconciliation_one_open_case_per_attempt
    ON payments.payment_reconciliation_cases (
        tenant_id, environment, payment_attempt_id
    )
    WHERE status IN ('pending', 'leased', 'manual_review');

CREATE INDEX payment_reconciliation_claim_idx
    ON payments.payment_reconciliation_cases (next_attempt_at, created_at)
    WHERE status = 'pending';

ALTER TABLE payments.payment_reconciliation_cases ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.payment_reconciliation_cases FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_reconciliation_cases_tenant_isolation
    ON payments.payment_reconciliation_cases
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);

CREATE TABLE payments.payment_reconciliation_attempts (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
    reconciliation_case_id UUID NOT NULL,
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    result_status TEXT NOT NULL CHECK (
        result_status IN (
            'not_found', 'processing', 'requires_action', 'authorized',
            'succeeded', 'failed', 'inconclusive', 'transport_error'
        )
    ),
    provider_transaction_reference TEXT,
    evidence_sha256 BYTEA CHECK (
        evidence_sha256 IS NULL OR octet_length(evidence_sha256) = 32
    ),
    safe_error_code TEXT,
    queried_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    UNIQUE (tenant_id, environment, id),
    UNIQUE (tenant_id, environment, reconciliation_case_id, attempt_number),
    FOREIGN KEY (tenant_id, environment, reconciliation_case_id)
        REFERENCES payments.payment_reconciliation_cases (tenant_id, environment, id)
);

CREATE INDEX payment_reconciliation_attempts_history_idx
    ON payments.payment_reconciliation_attempts (
        tenant_id, environment, reconciliation_case_id, queried_at DESC
    );

ALTER TABLE payments.payment_reconciliation_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE payments.payment_reconciliation_attempts FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_reconciliation_attempts_tenant_isolation
    ON payments.payment_reconciliation_attempts
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);
