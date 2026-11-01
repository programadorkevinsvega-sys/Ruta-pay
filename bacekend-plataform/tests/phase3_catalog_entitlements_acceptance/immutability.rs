use super::fixture::TestFixture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub async fn verify(fixture: &TestFixture) {
    let pool = &fixture.database.runtime_pool;
    let mut operator = pool.begin().await.expect("operator transaction begins");
    sqlx::query("SELECT set_config('app.operator_scope','true',true)")
        .execute(&mut *operator).await.expect("operator RLS scope is enabled");
    assert_rejected(&mut operator, "UPDATE catalog_pricing.price_versions SET amount_minor=1 WHERE id=$1", fixture.price_one.id).await;
    assert_rejected(&mut operator, "DELETE FROM catalog_pricing.price_versions WHERE id=$1", fixture.price_one.id).await;
    assert_rejected(&mut operator, "UPDATE entitlements_quotas.plan_capabilities SET enabled=false WHERE price_version_id=$1 AND capability_key='payments.monthly'", fixture.price_one.id).await;
    assert_rejected(&mut operator, "DELETE FROM entitlements_quotas.plan_capabilities WHERE price_version_id=$1 AND capability_key='payments.monthly'", fixture.price_one.id).await;
    operator.rollback().await.expect("operator verification transaction rolls back");

    let mut tenant = pool.begin().await.expect("tenant transaction begins");
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(fixture.tenant_one.to_string())
        .execute(&mut *tenant).await.expect("tenant RLS scope is enabled");
    assert_rejected(&mut tenant, "UPDATE entitlements_quotas.usage_ledger SET quantity=quantity+1 WHERE tenant_id=$1 AND idempotency_key='phase3-payment-001'", fixture.tenant_one).await;
    assert_rejected(&mut tenant, "DELETE FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1 AND idempotency_key='phase3-payment-001'", fixture.tenant_one).await;
    tenant.rollback().await.expect("tenant verification transaction rolls back");
}

async fn assert_rejected(
    tx: &mut Transaction<'_, Postgres>,
    statement: &'static str,
    id: Uuid,
) {
    let error = sqlx::query(statement).bind(id).execute(&mut **tx).await
        .expect_err("immutable records reject mutation at the PostgreSQL boundary");
    assert_eq!(error.as_database_error().and_then(|db| db.code()).as_deref(), Some("55000"));
}
