use platform_entitlements_quotas::{
    application::entitlement_service::ConsumeQuotaCommand, domain::entitlement::EntitlementError,
};

use super::fixture::{TestFixture, is_domain_error};

pub async fn verify(fixture: &TestFixture) {
    let entitlements = &fixture.entitlements;
    let actor = fixture.actor;
    let tenant_two = fixture.tenant_two;
    let now = fixture.now;
    let correlation = fixture.correlation;
    let (concurrent_a, concurrent_b) = tokio::join!(
        entitlements.consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_two,
            key: "payments.monthly",
            environment: "test",
            quantity: 3,
            idempotency_key: "phase3-race-a",
            now,
            correlation_id: correlation,
        }),
        entitlements.consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_two,
            key: "payments.monthly",
            environment: "test",
            quantity: 3,
            idempotency_key: "phase3-race-b",
            now,
            correlation_id: correlation,
        }),
    );
    let concurrent_successes =
        (if concurrent_a.is_ok() { 1 } else { 0 }) + (if concurrent_b.is_ok() { 1 } else { 0 });
    assert_eq!(
        concurrent_successes, 1,
        "only one concurrent transaction may consume three units from a five-unit quota"
    );
    let rejected = if let Err(error) = concurrent_a {
        error
    } else {
        concurrent_b.expect_err("one concurrent quota transaction is rejected")
    };
    assert!(is_domain_error(rejected, EntitlementError::QuotaExceeded));
    let concurrent_usage = entitlements
        .evaluate(actor, tenant_two, "payments.monthly", "test", now)
        .await
        .expect("concurrent usage remains queryable");
    assert_eq!(concurrent_usage.consumed, 3);

}
