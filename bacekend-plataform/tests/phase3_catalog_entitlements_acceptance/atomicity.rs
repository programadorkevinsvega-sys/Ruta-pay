use super::fixture::TestFixture;
use chrono::Duration;
use platform_catalog_pricing::application::catalog_service::PublishPriceVersionCommand;
use platform_entitlements_quotas::application::entitlement_service::ConsumeQuotaCommand;

pub async fn verify(fixture: &TestFixture) {
    verify_catalog_rollback(fixture).await;
    verify_usage_rollback(fixture).await;
}

async fn verify_catalog_rollback(fixture: &TestFixture) {
    let pool = &fixture.database.migration_pool;
    let plan = fixture
        .catalog
        .create_plan(
            fixture.actor,
            &format!("rollback-{}", uuid::Uuid::new_v4().simple()),
            "Rollback acceptance",
            "plan used to prove transaction rollback",
            fixture.correlation,
        )
        .await
        .expect("isolated rollback plan is created before failure injection");
    sqlx::raw_sql("CREATE FUNCTION catalog_pricing.reject_test_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'acceptance outbox failure' USING ERRCODE='23514'; END $$; CREATE TRIGGER phase3_reject_catalog_outbox BEFORE INSERT ON catalog_pricing.outbox_events FOR EACH ROW EXECUTE FUNCTION catalog_pricing.reject_test_outbox()")
        .execute(pool).await.expect("catalog failure injector is installed");
    let before: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM catalog_pricing.price_versions),(SELECT count(*) FROM catalog_pricing.audit_events)")
        .fetch_one(pool).await.expect("catalog state before failure is measurable");
    let result = fixture
        .catalog
        .publish_price_version(PublishPriceVersionCommand {
            actor_id: fixture.actor,
            plan_id: plan.id,
            currency: "USD",
            amount_minor: 9900,
            period_unit: "month",
            interval_count: 1,
            effective_from: fixture.now + Duration::days(365),
            effective_until: None,
            correlation_id: fixture.correlation,
        })
        .await;
    sqlx::raw_sql("DROP TRIGGER phase3_reject_catalog_outbox ON catalog_pricing.outbox_events; DROP FUNCTION catalog_pricing.reject_test_outbox()")
        .execute(pool).await.expect("catalog failure injector is removed");
    assert!(result.is_err(), "outbox failure aborts price publication");
    let after: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM catalog_pricing.price_versions),(SELECT count(*) FROM catalog_pricing.audit_events)")
        .fetch_one(pool).await.expect("catalog state after failure is measurable");
    assert_eq!(
        before, after,
        "price and audit writes roll back with the outbox"
    );
}

async fn verify_usage_rollback(fixture: &TestFixture) {
    let pool = &fixture.database.migration_pool;
    sqlx::raw_sql("CREATE FUNCTION entitlements_quotas.reject_test_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'acceptance outbox failure' USING ERRCODE='23514'; END $$; CREATE TRIGGER phase3_reject_entitlements_outbox BEFORE INSERT ON entitlements_quotas.outbox_events FOR EACH ROW EXECUTE FUNCTION entitlements_quotas.reject_test_outbox()")
        .execute(pool).await.expect("entitlement failure injector is installed");
    let before: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM entitlements_quotas.usage_ledger),(SELECT count(*) FROM entitlements_quotas.audit_events)")
        .fetch_one(pool).await.expect("usage state before failure is measurable");
    let result = fixture
        .entitlements
        .consume(ConsumeQuotaCommand {
            actor: fixture.actor,
            tenant: fixture.tenant_two,
            key: "payments.monthly",
            environment: "test",
            quantity: 1,
            idempotency_key: "phase3-outbox-rollback",
            now: fixture.now,
            correlation_id: fixture.correlation,
        })
        .await;
    sqlx::raw_sql("DROP TRIGGER phase3_reject_entitlements_outbox ON entitlements_quotas.outbox_events; DROP FUNCTION entitlements_quotas.reject_test_outbox()")
        .execute(pool).await.expect("entitlement failure injector is removed");
    assert!(result.is_err(), "outbox failure aborts quota consumption");
    let after: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM entitlements_quotas.usage_ledger),(SELECT count(*) FROM entitlements_quotas.audit_events)")
        .fetch_one(pool).await.expect("usage state after failure is measurable");
    assert_eq!(
        before, after,
        "usage and audit writes roll back with the outbox"
    );
}
