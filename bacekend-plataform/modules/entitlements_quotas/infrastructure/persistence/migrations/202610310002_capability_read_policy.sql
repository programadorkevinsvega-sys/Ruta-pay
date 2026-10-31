CREATE POLICY entitlement_capabilities_public_read ON entitlements_quotas.plan_capabilities
    FOR SELECT USING (true);
