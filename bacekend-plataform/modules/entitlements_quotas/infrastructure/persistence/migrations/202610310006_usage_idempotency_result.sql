-- Preserve the exact response produced by a quota consumption so an idempotent
-- replay remains stable even after later usage changes or plan reassignment.
ALTER TABLE entitlements_quotas.usage_ledger
    ADD COLUMN result_snapshot JSONB;

ALTER TABLE entitlements_quotas.usage_ledger
    ADD CONSTRAINT usage_ledger_result_snapshot_object
    CHECK (result_snapshot IS NULL OR jsonb_typeof(result_snapshot) = 'object');

CREATE FUNCTION entitlements_quotas.require_idempotency_result_snapshot()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.result_snapshot IS NULL OR jsonb_typeof(NEW.result_snapshot) <> 'object' THEN
        RAISE EXCEPTION 'usage ledger entries require an immutable idempotency result snapshot'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER usage_ledger_result_snapshot_required
    BEFORE INSERT ON entitlements_quotas.usage_ledger
    FOR EACH ROW EXECUTE FUNCTION entitlements_quotas.require_idempotency_result_snapshot();
