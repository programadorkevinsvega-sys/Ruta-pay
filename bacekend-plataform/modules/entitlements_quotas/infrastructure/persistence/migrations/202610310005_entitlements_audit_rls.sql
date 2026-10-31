ALTER TABLE entitlements_quotas.audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE entitlements_quotas.audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY entitlements_audit_scope ON entitlements_quotas.audit_events
    USING (tenant_id=nullif(current_setting('app.tenant_id',true),'')::uuid OR current_setting('app.operator_scope',true)='true')
    WITH CHECK (tenant_id=nullif(current_setting('app.tenant_id',true),'')::uuid OR current_setting('app.operator_scope',true)='true');
