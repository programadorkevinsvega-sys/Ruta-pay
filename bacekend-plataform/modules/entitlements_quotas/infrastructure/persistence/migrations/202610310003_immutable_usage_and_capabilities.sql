CREATE FUNCTION entitlements_quotas.reject_capability_mutation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'capability definitions are immutable for a published price version' USING ERRCODE='55000';
END $$;
CREATE TRIGGER plan_capabilities_immutable
    BEFORE UPDATE OR DELETE ON entitlements_quotas.plan_capabilities
    FOR EACH ROW EXECUTE FUNCTION entitlements_quotas.reject_capability_mutation();

CREATE FUNCTION entitlements_quotas.protect_usage_ledger()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'usage ledger is append-only' USING ERRCODE='55000';
END $$;
CREATE TRIGGER usage_ledger_append_only
    BEFORE UPDATE OR DELETE ON entitlements_quotas.usage_ledger
    FOR EACH ROW EXECUTE FUNCTION entitlements_quotas.protect_usage_ledger();
