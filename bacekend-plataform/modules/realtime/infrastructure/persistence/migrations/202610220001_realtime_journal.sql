CREATE SCHEMA IF NOT EXISTS realtime;

CREATE TABLE realtime.event_journal (
    cursor BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type TEXT NOT NULL,
    schema_version SMALLINT NOT NULL CHECK (schema_version > 0),
    aggregate_id UUID NOT NULL,
    aggregate_version BIGINT NOT NULL CHECK (aggregate_version > 0),
    audience TEXT NOT NULL CHECK (audience IN ('tenant', 'tenant_and_platform', 'platform_operator')),
    tenant_id UUID,
    tenant_cursor BIGINT,
    platform_cursor BIGINT,
    correlation_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((audience IN ('tenant', 'tenant_and_platform')) = (tenant_id IS NOT NULL)),
    CHECK ((audience IN ('tenant', 'tenant_and_platform')) = (tenant_cursor IS NOT NULL)),
    CHECK ((audience IN ('tenant_and_platform', 'platform_operator')) = (platform_cursor IS NOT NULL))
);

CREATE TABLE realtime.scope_cursors (
    scope_key TEXT PRIMARY KEY,
    cursor BIGINT NOT NULL CHECK (cursor > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX realtime_journal_tenant_cursor_idx
    ON realtime.event_journal (tenant_id, tenant_cursor)
    WHERE tenant_cursor IS NOT NULL;
CREATE INDEX realtime_journal_platform_cursor_idx
    ON realtime.event_journal (platform_cursor)
    WHERE platform_cursor IS NOT NULL;

ALTER TABLE realtime.event_journal ENABLE ROW LEVEL SECURITY;
ALTER TABLE realtime.event_journal FORCE ROW LEVEL SECURITY;

CREATE POLICY realtime_journal_scoped_read ON realtime.event_journal
    FOR SELECT
    USING (
        (current_setting('app.realtime_audience', true) = 'platform'
            AND audience IN ('tenant_and_platform', 'platform_operator'))
        OR
        (current_setting('app.realtime_audience', true) = 'tenant'
            AND tenant_id = nullif(current_setting('app.realtime_tenant_id', true), '')::UUID
            AND audience IN ('tenant', 'tenant_and_platform'))
    );

CREATE POLICY realtime_journal_consumer_insert ON realtime.event_journal
    FOR INSERT
    WITH CHECK (true);
