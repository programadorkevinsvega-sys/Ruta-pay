use platform_entitlements_quotas::{
    application::entitlement_service::ConsumeQuotaCommand, domain::entitlement::EntitlementError,
};

use super::fixture::{TestFixture, is_domain_error};

pub async fn verify(fixture: &TestFixture) {
    let entitlements = &fixture.entitlements;
    let actor = fixture.actor;
    let tenant_one = fixture.tenant_one;
    let now = fixture.now;
    let correlation = fixture.correlation;
    let initial = entitlements
        .evaluate(actor, tenant_one, "payments.monthly", "test", now)
        .await
        .expect("tenant entitlement evaluates");
    assert!(initial.allowed);
    assert_eq!(initial.limit_value, Some(5));
    assert_eq!(initial.consumed, 0);
    assert_eq!(initial.remaining, Some(5));

    let (after_four, replayed) = entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_one,
            key: "payments.monthly",
            environment: "test",
            quantity: 4,
            idempotency_key: "phase3-payment-001",
            now,
            correlation_id: correlation,
        })
        .await
        .expect("usage below the quota is accepted");
    assert!(!replayed);
    assert_eq!(after_four.consumed, 4);

    let live_before = entitlements
        .evaluate(actor, tenant_one, "payments.monthly", "live", now)
        .await
        .expect("live usage is isolated from test usage");
    assert_eq!(live_before.consumed, 0);
    assert_eq!(live_before.remaining, Some(5));
    let (live_usage, _) = entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_one,
            key: "payments.monthly",
            environment: "live",
            quantity: 2,
            idempotency_key: "phase3-live-payment-001",
            now,
            correlation_id: correlation,
        })
        .await
        .expect("live usage consumes from its independent quota");
    assert_eq!(live_usage.consumed, 2);
    assert_eq!(live_usage.remaining, Some(3));

    let (replayed_result, replayed) = entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_one,
            key: "payments.monthly",
            environment: "test",
            quantity: 4,
            idempotency_key: "phase3-payment-001",
            now,
            correlation_id: correlation,
        })
        .await
        .expect("identical idempotent replay returns the recorded result");
    assert!(replayed);
    assert_eq!(replayed_result.consumed, 4);
    let conflict = entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_one,
            key: "payments.monthly",
            environment: "test",
            quantity: 3,
            idempotency_key: "phase3-payment-001",
            now,
            correlation_id: correlation,
        })
        .await
        .expect_err("idempotency key cannot be reused with different operation data");
    assert!(is_domain_error(
        conflict,
        EntitlementError::IdempotencyConflict
    ));

    let (at_limit, replayed) = entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_one,
            key: "payments.monthly",
            environment: "test",
            quantity: 1,
            idempotency_key: "phase3-payment-002",
            now,
            correlation_id: correlation,
        })
        .await
        .expect("consumption exactly at the limit is accepted");
    assert!(!replayed);
    assert_eq!(at_limit.consumed, 5);
    assert!(!at_limit.allowed);
    assert_eq!(at_limit.remaining, Some(0));
    let exceeded = entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_one,
            key: "payments.monthly",
            environment: "test",
            quantity: 1,
            idempotency_key: "phase3-payment-003",
            now,
            correlation_id: correlation,
        })
        .await
        .expect_err("consumption above the limit is rejected");
    assert!(is_domain_error(exceeded, EntitlementError::QuotaExceeded));

    let (overage, _) = entitlements
        .consume(ConsumeQuotaCommand {
            actor,
            tenant: tenant_one,
            key: "reports.export",
            environment: "test",
            quantity: 2,
            idempotency_key: "phase3-overage-001",
            now,
            correlation_id: correlation,
        })
        .await
        .expect("allow-overage policy records usage above the configured limit");
    assert!(overage.allowed);
    assert_eq!(overage.consumed, 2);
    assert_eq!(overage.remaining, Some(0));

    let restricted = entitlements
        .evaluate(actor, tenant_one, "live.feature", "test", now)
        .await
        .expect("environment-specific capability is evaluated");
    assert!(!restricted.allowed);
    assert_eq!(restricted.reason, "environment_restricted");
    assert!(
        entitlements
            .evaluate(actor, tenant_one, "live.feature", "live", now)
            .await
            .expect("live environment is accepted")
            .allowed
    );

}
