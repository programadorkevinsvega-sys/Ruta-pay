CREATE SCHEMA IF NOT EXISTS platform_billing;

CREATE TABLE platform_billing.usage_events (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    meter_key TEXT NOT NULL CHECK (meter_key ~ '^[a-z][a-z0-9._-]{0,95}$'),
    source_event_id TEXT NOT NULL CHECK (char_length(source_event_id) BETWEEN 1 AND 160),
    source_system TEXT NOT NULL CHECK (source_system ~ '^[a-z][a-z0-9._-]{0,63}$'),
    quantity BIGINT NOT NULL CHECK (quantity > 0),
    unit TEXT NOT NULL CHECK (char_length(unit) BETWEEN 1 AND 32),
    occurred_at TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attributes JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(attributes)='object'),
    UNIQUE (tenant_id,source_system,source_event_id),
    UNIQUE (tenant_id,id)
);
CREATE INDEX usage_events_billing_period_idx
    ON platform_billing.usage_events(tenant_id,meter_key,occurred_at,id);

CREATE TABLE platform_billing.invoices (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    invoice_number TEXT NOT NULL CHECK (char_length(invoice_number) BETWEEN 1 AND 64),
    status TEXT NOT NULL CHECK (status IN ('draft','issued','voided')),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL,
    issued_at TIMESTAMPTZ,
    due_at TIMESTAMPTZ,
    subtotal_minor BIGINT NOT NULL DEFAULT 0 CHECK (subtotal_minor >= 0),
    total_minor BIGINT NOT NULL DEFAULT 0 CHECK (total_minor >= 0),
    snapshot_version SMALLINT,
    snapshot JSONB,
    snapshot_sha256 BYTEA,
    aggregate_version BIGINT NOT NULL DEFAULT 1 CHECK (aggregate_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,id),
    UNIQUE (tenant_id,invoice_number),
    CHECK (period_end > period_start),
    CHECK (total_minor = subtotal_minor),
    CHECK (
      (status='draft' AND issued_at IS NULL AND snapshot IS NULL AND snapshot_sha256 IS NULL AND snapshot_version IS NULL)
      OR
      (status IN ('issued','voided') AND issued_at IS NOT NULL AND snapshot IS NOT NULL
       AND jsonb_typeof(snapshot)='object' AND snapshot_sha256 IS NOT NULL AND octet_length(snapshot_sha256)=32 AND snapshot_version IS NOT NULL AND snapshot_version > 0)
    )
);
CREATE INDEX invoices_tenant_period_idx ON platform_billing.invoices(tenant_id,period_start DESC);

CREATE TABLE platform_billing.invoice_lines (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    invoice_id UUID NOT NULL,
    line_number INTEGER NOT NULL CHECK (line_number > 0),
    source_usage_event_id UUID,
    description TEXT NOT NULL CHECK (char_length(description) BETWEEN 1 AND 500),
    meter_key TEXT,
    quantity BIGINT NOT NULL CHECK (quantity > 0),
    unit TEXT NOT NULL CHECK (char_length(unit) BETWEEN 1 AND 32),
    unit_price_minor BIGINT NOT NULL CHECK (unit_price_minor >= 0),
    line_total_minor BIGINT NOT NULL CHECK (line_total_minor >= 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,invoice_id,line_number),
    UNIQUE (tenant_id,id),
    FOREIGN KEY (tenant_id,invoice_id) REFERENCES platform_billing.invoices(tenant_id,id),
    FOREIGN KEY (tenant_id,source_usage_event_id) REFERENCES platform_billing.usage_events(tenant_id,id),
    CHECK (line_total_minor = quantity * unit_price_minor)
);
CREATE INDEX invoice_lines_invoice_idx ON platform_billing.invoice_lines(tenant_id,invoice_id,line_number);

CREATE TABLE platform_billing.ledger_journals (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    invoice_id UUID NOT NULL,
    journal_key TEXT NOT NULL CHECK (char_length(journal_key) BETWEEN 1 AND 160),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,id),
    UNIQUE (tenant_id,journal_key),
    FOREIGN KEY (tenant_id,invoice_id) REFERENCES platform_billing.invoices(tenant_id,id)
);
CREATE TABLE platform_billing.ledger_entries (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    journal_id UUID NOT NULL,
    entry_number SMALLINT NOT NULL CHECK (entry_number > 0),
    account_code TEXT NOT NULL CHECK (account_code ~ '^[a-z][a-z0-9._-]{0,63}$'),
    direction TEXT NOT NULL CHECK (direction IN ('debit','credit')),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,journal_id,entry_number),
    FOREIGN KEY (tenant_id,journal_id) REFERENCES platform_billing.ledger_journals(tenant_id,id)
);
CREATE INDEX ledger_entries_journal_idx ON platform_billing.ledger_entries(tenant_id,journal_id);

CREATE FUNCTION platform_billing.assert_balanced_journal()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE journal_key UUID;
DECLARE debit_total NUMERIC;
DECLARE credit_total NUMERIC;
BEGIN
    journal_key := COALESCE(NEW.journal_id, OLD.journal_id);
    SELECT COALESCE(sum(amount_minor) FILTER (WHERE direction='debit'),0),
           COALESCE(sum(amount_minor) FILTER (WHERE direction='credit'),0)
      INTO debit_total,credit_total
      FROM platform_billing.ledger_entries WHERE journal_id=journal_key;
    IF debit_total=0 OR debit_total<>credit_total THEN
        RAISE EXCEPTION 'ledger journal must contain balanced debit and credit entries'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER ledger_journal_balanced
    AFTER INSERT ON platform_billing.ledger_entries
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION platform_billing.assert_balanced_journal();

CREATE FUNCTION platform_billing.reject_financial_mutation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'issued billing records and ledger are immutable'
        USING ERRCODE='55000';
END $$;
CREATE TRIGGER invoice_lines_immutable
    BEFORE UPDATE OR DELETE ON platform_billing.invoice_lines
    FOR EACH ROW EXECUTE FUNCTION platform_billing.reject_financial_mutation();
CREATE TRIGGER ledger_journals_immutable
    BEFORE UPDATE OR DELETE ON platform_billing.ledger_journals
    FOR EACH ROW EXECUTE FUNCTION platform_billing.reject_financial_mutation();
CREATE TRIGGER ledger_entries_immutable
    BEFORE UPDATE OR DELETE ON platform_billing.ledger_entries
    FOR EACH ROW EXECUTE FUNCTION platform_billing.reject_financial_mutation();
CREATE FUNCTION platform_billing.protect_issued_invoice()
RETURNS trigger LANGUAGE plpgsql AS $
BEGIN
    IF TG_OP='DELETE' THEN
        IF OLD.status IN ('issued','voided') THEN
            RAISE EXCEPTION 'issued invoice is immutable; use a compensating operation'
                USING ERRCODE='55000';
        END IF;
        RETURN OLD;
    END IF;
    IF OLD.status IN ('issued','voided') AND (
        NEW.tenant_id IS DISTINCT FROM OLD.tenant_id OR
        NEW.invoice_number IS DISTINCT FROM OLD.invoice_number OR
        NEW.currency IS DISTINCT FROM OLD.currency OR
        NEW.period_start IS DISTINCT FROM OLD.period_start OR
        NEW.period_end IS DISTINCT FROM OLD.period_end OR
        NEW.issued_at IS DISTINCT FROM OLD.issued_at OR
        NEW.subtotal_minor IS DISTINCT FROM OLD.subtotal_minor OR
        NEW.total_minor IS DISTINCT FROM OLD.total_minor OR
        NEW.snapshot_version IS DISTINCT FROM OLD.snapshot_version OR
        NEW.snapshot IS DISTINCT FROM OLD.snapshot OR
        NEW.snapshot_sha256 IS DISTINCT FROM OLD.snapshot_sha256
    ) THEN
        RAISE EXCEPTION 'issued invoice financial content is immutable'
            USING ERRCODE='55000';
    END IF;
    IF OLD.status='voided' AND NEW.status<>'voided' THEN
        RAISE EXCEPTION 'voided invoice cannot be reopened'
            USING ERRCODE='55000';
    END IF;
    RETURN NEW;
END $;
CREATE TRIGGER issued_invoice_immutable
    BEFORE UPDATE OR DELETE ON platform_billing.invoices
    FOR EACH ROW EXECUTE FUNCTION platform_billing.protect_issued_invoice();
CREATE TRIGGER usage_events_immutable
    BEFORE UPDATE OR DELETE ON platform_billing.usage_events
    FOR EACH ROW EXECUTE FUNCTION platform_billing.reject_financial_mutation();

CREATE TABLE platform_billing.outbox_events (
    event_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    aggregate_id UUID NOT NULL,
    aggregate_version BIGINT NOT NULL CHECK (aggregate_version > 0),
    event_type TEXT NOT NULL CHECK (event_type ~ '^[a-z][a-z0-9._-]{0,127}$'),
    schema_version SMALLINT NOT NULL CHECK (schema_version > 0),
    audience TEXT NOT NULL CHECK (audience IN ('platform','fiscal_integration','tenant')),
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload)='object'),
    correlation_id TEXT NOT NULL CHECK (char_length(correlation_id) BETWEEN 1 AND 128),
    traceparent TEXT,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    last_error_code TEXT,
    UNIQUE (aggregate_id,aggregate_version,event_type),
    CHECK ((lease_token IS NULL)=(lease_until IS NULL))
);
CREATE INDEX billing_outbox_pending_idx ON platform_billing.outbox_events(available_at,occurred_at)
    WHERE published_at IS NULL AND failed_at IS NULL;

CREATE TABLE platform_billing.inbox_events (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    producer_id TEXT NOT NULL CHECK (char_length(producer_id) BETWEEN 1 AND 256),
    event_id UUID NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type ~ '^[a-z][a-z0-9._-]{0,127}$'),
    schema_version SMALLINT NOT NULL CHECK (schema_version > 0),
    payload_sha256 BYTEA NOT NULL CHECK (octet_length(payload_sha256)=32),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    disposition TEXT NOT NULL CHECK (disposition IN ('received','processed','rejected')),
    rejection_code TEXT,
    UNIQUE (tenant_id,producer_id,event_id),
    UNIQUE (tenant_id,id),
    CHECK ((disposition='rejected')=(rejection_code IS NOT NULL))
);
CREATE INDEX billing_inbox_unprocessed_idx ON platform_billing.inbox_events(received_at)
    WHERE disposition='received';

-- SaaS collection belongs to platform billing. It deliberately has no FK to
-- payments.payment_intents/attempts, which remain tenant-buyer payment records.
CREATE TABLE platform_billing.collection_attempts (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    invoice_id UUID NOT NULL,
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    provider_code TEXT NOT NULL CHECK (provider_code ~ '^[a-z][a-z0-9_]{1,31}$'),
    provider_reference TEXT,
    idempotency_key TEXT NOT NULL CHECK (char_length(idempotency_key) BETWEEN 1 AND 128),
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256)=32),
    status TEXT NOT NULL CHECK (status IN ('created','submitted','processing','succeeded','failed','outcome_unknown')),
    aggregate_version BIGINT NOT NULL DEFAULT 1 CHECK (aggregate_version > 0),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,id),
    UNIQUE (tenant_id,invoice_id),
    UNIQUE (tenant_id,invoice_id,idempotency_key),
    FOREIGN KEY (tenant_id,invoice_id) REFERENCES platform_billing.invoices(tenant_id,id)
);

CREATE FUNCTION platform_billing.guard_collection_attempt()
RETURNS trigger LANGUAGE plpgsql AS $
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id OR
       NEW.invoice_id IS DISTINCT FROM OLD.invoice_id OR
       NEW.attempt_number IS DISTINCT FROM OLD.attempt_number OR
       NEW.provider_code IS DISTINCT FROM OLD.provider_code OR
       NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key OR
       NEW.request_sha256 IS DISTINCT FROM OLD.request_sha256 OR
       NEW.amount_minor IS DISTINCT FROM OLD.amount_minor OR
       NEW.currency IS DISTINCT FROM OLD.currency THEN
        RAISE EXCEPTION 'collection attempt request is immutable'
            USING ERRCODE='55000';
    END IF;
    IF OLD.status IN ('succeeded','failed') OR NOT (
        (OLD.status='created' AND NEW.status IN ('submitted','failed','outcome_unknown')) OR
        (OLD.status='submitted' AND NEW.status IN ('processing','succeeded','failed','outcome_unknown')) OR
        (OLD.status='processing' AND NEW.status IN ('succeeded','failed','outcome_unknown')) OR
        (OLD.status='outcome_unknown' AND NEW.status IN ('processing','succeeded','failed'))
    ) THEN
        RAISE EXCEPTION 'invalid platform collection state transition'
            USING ERRCODE='23514';
    END IF;
    NEW.aggregate_version := OLD.aggregate_version + 1;
    NEW.updated_at := now();
    RETURN NEW;
END $;
CREATE TRIGGER collection_attempt_state_guard
    BEFORE UPDATE ON platform_billing.collection_attempts
    FOR EACH ROW EXECUTE FUNCTION platform_billing.guard_collection_attempt();

DO $
DECLARE table_name TEXT;
BEGIN
  FOREACH table_name IN ARRAY ARRAY[
    'usage_events','invoices','invoice_lines','ledger_journals',
    'ledger_entries','outbox_events','inbox_events','collection_attempts'
  ] LOOP
    EXECUTE format('ALTER TABLE platform_billing.%I ENABLE ROW LEVEL SECURITY',table_name);
    EXECUTE format('ALTER TABLE platform_billing.%I FORCE ROW LEVEL SECURITY',table_name);
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON platform_billing.%I USING (tenant_id=nullif(current_setting(''app.tenant_id'',true),'''')::uuid OR current_setting(''app.operator_scope'',true)=''true'') WITH CHECK (tenant_id=nullif(current_setting(''app.tenant_id'',true),'''')::uuid OR current_setting(''app.operator_scope'',true)=''true'')',
      table_name
    );
  END LOOP;
END $$;
