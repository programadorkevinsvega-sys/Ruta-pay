CREATE SCHEMA IF NOT EXISTS entitlements_quotas;

CREATE TABLE entitlements_quotas.plan_capabilities (
    price_version_id UUID NOT NULL,
    capability_key TEXT NOT NULL CHECK (capability_key ~ '^[a-z0-9][a-z0-9._-]{0,95}$'),
    enabled BOOLEAN NOT NULL,
    limit_value BIGINT CHECK (limit_value IS NULL OR limit_value >= 0),
    unit TEXT NOT NULL CHECK (char_length(unit) BETWEEN 1 AND 32),
    overage_policy TEXT NOT NULL CHECK (overage_policy IN ('deny','allow')),
    environment TEXT NOT NULL CHECK (environment IN ('all','test','live')),
    quota_period TEXT NOT NULL CHECK (quota_period IN ('day','week','month','year')),
    created_by UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (price_version_id,capability_key)
);

CREATE TABLE entitlements_quotas.tenant_plan_assignments (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    price_version_id UUID NOT NULL,
    effective_from TIMESTAMPTZ NOT NULL,
    effective_until TIMESTAMPTZ,
    assigned_by UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (effective_until IS NULL OR effective_until > effective_from)
);
CREATE INDEX tenant_plan_assignments_effective_idx
    ON entitlements_quotas.tenant_plan_assignments(tenant_id,effective_from,effective_until);

CREATE TABLE entitlements_quotas.usage_ledger (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    price_version_id UUID NOT NULL,
    capability_key TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('test','live')),
    idempotency_key TEXT NOT NULL CHECK (char_length(idempotency_key) BETWEEN 1 AND 128),
    quantity BIGINT NOT NULL CHECK (quantity > 0),
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,idempotency_key),
    CHECK (period_end > period_start)
);
CREATE INDEX usage_ledger_period_idx
    ON entitlements_quotas.usage_ledger(tenant_id,capability_key,period_start,period_end);

ALTER TABLE entitlements_quotas.plan_capabilities ENABLE ROW LEVEL SECURITY;
ALTER TABLE entitlements_quotas.plan_capabilities FORCE ROW LEVEL SECURITY;
CREATE POLICY entitlement_capabilities_operator ON entitlements_quotas.plan_capabilities
    USING (current_setting('app.operator_scope',true)='true')
    WITH CHECK (current_setting('app.operator_scope',true)='true');

ALTER TABLE entitlements_quotas.tenant_plan_assignments ENABLE ROW LEVEL SECURITY;
ALTER TABLE entitlements_quotas.tenant_plan_assignments FORCE ROW LEVEL SECURITY;
CREATE POLICY entitlement_assignments_scope ON entitlements_quotas.tenant_plan_assignments
    USING (tenant_id=nullif(current_setting('app.tenant_id',true),'')::uuid OR current_setting('app.operator_scope',true)='true')
    WITH CHECK (current_setting('app.operator_scope',true)='true');

ALTER TABLE entitlements_quotas.usage_ledger ENABLE ROW LEVEL SECURITY;
ALTER TABLE entitlements_quotas.usage_ledger FORCE ROW LEVEL SECURITY;
CREATE POLICY entitlement_usage_scope ON entitlements_quotas.usage_ledger
    USING (tenant_id=nullif(current_setting('app.tenant_id',true),'')::uuid)
    WITH CHECK (tenant_id=nullif(current_setting('app.tenant_id',true),'')::uuid);
