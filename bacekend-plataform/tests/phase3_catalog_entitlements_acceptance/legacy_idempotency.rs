use chrono::Duration;
use platform_entitlements_quotas::application::entitlement_service::{
    ConsumeQuotaCommand, EntitlementServiceError,
};
use uuid::Uuid;

use super::fixture::TestFixture;

pub async fn verify(fixture: &TestFixture) {
    let tenant = fixture.tenant_one;
    let actor = fixture.actor;
    let price_version = fixture.price_one.id;
    let now = fixture.now;
    let mut transaction = fixture
        .database
        .migration_pool
        .begin()
        .await
        .expect("isolated database owner opens a transaction");
    sqlx::query("SELECT set_config('app.tenant_id',$1,true),set_config('app.actor_id',$2,true)")
        .bind(tenant.to_string())
        .bind(actor.to_string())
        .execute(&mut *transaction)
        .await
        .expect("legacy row is scoped to its tenant");
    sqlx::query("ALTER TABLE entitlements_quotas.usage_ledger DISABLE TRIGGER usage_ledger_result_snapshot_required")
        .execute(&mut *transaction)
        .await
        .expect("only the isolated acceptance database disables the new-row guard");
    sqlx::query("INSERT INTO entitlements_quotas.usage_ledger (id,tenant_id,price_version_id,capability_key,environment,idempotency_key,quantity,period_start,period_end) VALUES ($1,$2,$3,'payments.monthly','test','legacy-phase3-idempotency',1,$4,$5)")
        .bind(Uuid::new_v4())
        .bind(tenant)
        .bind(price_version)
        .bind(now - Duration::days(1))
        .bind(now + Duration::days(29))
        .execute(&mut *transaction)
        .await
        .expect("pre-migration usage row is represented without a response snapshot");
    sqlx::query("ALTER TABLE entitlements_quotas.usage_ledger ENABLE TRIGGER usage_ledger_result_snapshot_required")
        .execute(&mut *transaction)
        .await
        .expect("new ledger writes are protected again");
    transaction
        .commit()
        .await
        .expect("legacy row is committed only in the isolated acceptance database");

    let error = fixture
        .entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant,
            key: "payments.monthly",
            environment: "test",
            quantity: 1,
            idempotency_key: "legacy-phase3-idempotency",
            now,
            correlation_id: fixture.correlation,
        })
        .await
        .expect_err("legacy replay without an original response fails closed");
    assert!(
        matches!(error, EntitlementServiceError::IdempotencyResultUnavailable),
        "legacy replay must not return a recomputed current entitlement"
    );
}
