ALTER TABLE catalog_pricing.audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog_pricing.audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY catalog_audit_operator_read ON catalog_pricing.audit_events FOR SELECT
    USING (current_setting('app.operator_scope',true)='true');
CREATE POLICY catalog_audit_operator_write ON catalog_pricing.audit_events
    USING (current_setting('app.operator_scope',true)='true')
    WITH CHECK (current_setting('app.operator_scope',true)='true');
