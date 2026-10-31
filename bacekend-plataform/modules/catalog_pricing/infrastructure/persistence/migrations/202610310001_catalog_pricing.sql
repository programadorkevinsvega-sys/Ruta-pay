CREATE SCHEMA IF NOT EXISTS catalog_pricing;

CREATE TABLE catalog_pricing.plans (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$'),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 2 AND 100),
    description TEXT NOT NULL DEFAULT '' CHECK (char_length(description) <= 1000),
    status TEXT NOT NULL CHECK (status IN ('draft','active','retired')),
    created_by UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE catalog_pricing.price_versions (
    id UUID PRIMARY KEY,
    plan_id UUID NOT NULL REFERENCES catalog_pricing.plans(id),
    version INTEGER NOT NULL CHECK (version > 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    amount_minor BIGINT NOT NULL CHECK (amount_minor >= 0),
    period_unit TEXT NOT NULL CHECK (period_unit IN ('day','week','month','year')),
    interval_count INTEGER NOT NULL CHECK (interval_count BETWEEN 1 AND 365),
    effective_from TIMESTAMPTZ NOT NULL,
    effective_until TIMESTAMPTZ,
    published_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (plan_id, version),
    CHECK (effective_until IS NULL OR effective_until > effective_from)
);
CREATE INDEX price_versions_effective_idx
    ON catalog_pricing.price_versions(plan_id,effective_from,effective_until);

CREATE FUNCTION catalog_pricing.reject_price_version_mutation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'published price versions are immutable' USING ERRCODE='55000';
END $$;
CREATE TRIGGER price_versions_immutable
    BEFORE UPDATE OR DELETE ON catalog_pricing.price_versions
    FOR EACH ROW EXECUTE FUNCTION catalog_pricing.reject_price_version_mutation();

ALTER TABLE catalog_pricing.plans ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog_pricing.plans FORCE ROW LEVEL SECURITY;
CREATE POLICY catalog_plans_operator_read ON catalog_pricing.plans FOR SELECT
    USING (current_setting('app.operator_scope',true)='true');
CREATE POLICY catalog_plans_operator_write ON catalog_pricing.plans
    USING (current_setting('app.operator_scope',true)='true')
    WITH CHECK (current_setting('app.operator_scope',true)='true');

ALTER TABLE catalog_pricing.price_versions ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog_pricing.price_versions FORCE ROW LEVEL SECURITY;
CREATE POLICY catalog_prices_operator_read ON catalog_pricing.price_versions FOR SELECT
    USING (current_setting('app.operator_scope',true)='true');
CREATE POLICY catalog_prices_operator_write ON catalog_pricing.price_versions
    USING (current_setting('app.operator_scope',true)='true')
    WITH CHECK (current_setting('app.operator_scope',true)='true');
