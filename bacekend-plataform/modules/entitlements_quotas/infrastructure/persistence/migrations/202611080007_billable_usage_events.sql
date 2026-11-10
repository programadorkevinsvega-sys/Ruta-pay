-- Quota usage is not billable by default. An operator must explicitly
-- configure billable=true on an immutable price-version capability.
ALTER TABLE entitlements_quotas.plan_capabilities
    ADD COLUMN billable BOOLEAN NOT NULL DEFAULT FALSE;
