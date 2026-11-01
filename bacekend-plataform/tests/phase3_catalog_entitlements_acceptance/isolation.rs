use platform_entitlements_quotas::domain::entitlement::EntitlementError;
use super::{fixture::{TestFixture, is_domain_error}};

pub async fn verify(fixture: &TestFixture) {
    let entitlements = &fixture.entitlements;
    let actor = fixture.actor;
    let tenant_unassigned = fixture.tenant_unassigned;
    let tenant_one = fixture.tenant_one;
    let tenant_two = fixture.tenant_two;
    let runtime_pool = &fixture.database.runtime_pool;
    let migration_pool = &fixture.database.migration_pool;
        let unassigned = entitlements
            .evaluate(actor, tenant_unassigned, "payments.monthly", "test", now)
            .await
            .expect_err("another tenant cannot inherit a tenant's plan or usage");
        assert!(is_domain_error(
            unassigned,
            EntitlementError::CapabilityUnavailable
        ));
    
        let mut tenant_scope = runtime_pool
            .begin()
            .await
            .expect("tenant scope transaction begins");
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant_one.to_string())
            .execute(&mut *tenant_scope)
            .await
            .expect("tenant RLS context is set");
        let foreign_usage: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1",
        )
        .bind(tenant_two)
        .fetch_one(&mut *tenant_scope)
        .await
        .expect("foreign tenant ledger query executes under RLS");
        assert_eq!(foreign_usage, 0, "RLS hides another tenant's usage");
        let foreign_assignments: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM entitlements_quotas.tenant_plan_assignments WHERE tenant_id=$1",
        )
        .bind(tenant_two)
        .fetch_one(&mut *tenant_scope)
        .await
        .expect("foreign tenant assignment query executes under RLS");
        assert_eq!(
            foreign_assignments, 0,
            "RLS hides another tenant's assignments"
        );
        tenant_scope
            .rollback()
            .await
            .expect("tenant scope transaction rolls back");
    
        let audit_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM entitlements_quotas.audit_events")
                .fetch_one(&migration_pool)
                .await
                .expect("audit records are queryable by the migration role");
        let outbox_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM entitlements_quotas.outbox_events")
                .fetch_one(&migration_pool)
                .await
                .expect("outbox records are queryable by the migration role");
        assert!(
            audit_count >= 5,
            "sensitive entitlement and usage actions are audited"
        );
        assert!(
            outbox_count >= 5,
            "business events are committed with their state changes"
        );
    
}
