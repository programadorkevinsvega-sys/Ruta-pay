use super::fixture::{TestFixture, capability};
use chrono::Duration;
use platform_catalog_pricing::application::catalog_service::PublishPriceVersionCommand;

pub async fn verify(fixture: &TestFixture) {
    let catalog = &fixture.catalog;
    let entitlements = &fixture.entitlements;
    let actor = fixture.actor;
    let tenant_one = fixture.tenant_one;
    let plan = &fixture.plan;
    let price_one = &fixture.price_one;
    let now = fixture.now;
    let future_start = fixture.future_start;
    let correlation = fixture.correlation;
    let price_two = catalog
        .publish_price_version(PublishPriceVersionCommand {
            actor_id: actor,
            plan_id: plan.id,
            currency: "USD",
            amount_minor: 1500,
            period_unit: "month",
            interval_count: 1,
            effective_from: future_start,
            effective_until: None,
            correlation_id: correlation,
        })
        .await
        .expect("future price version is published without mutating the previous version");
    entitlements
        .set_capabilities(
            actor,
            price_two.id,
            &[capability("payments.monthly", Some(10), "all", "deny")],
            correlation,
        )
        .await
        .expect("new price version receives a new immutable capability snapshot");
    entitlements
        .assign_plan(
            actor,
            tenant_one,
            price_two.id,
            future_start,
            None,
            correlation,
        )
        .await
        .expect("future plan assignment closes the previous effective assignment");
    let before_change = entitlements
        .evaluate(actor, tenant_one, "payments.monthly", "test", now)
        .await
        .expect("historical plan remains effective before the scheduled change");
    let after_change = entitlements
        .evaluate(
            actor,
            tenant_one,
            "payments.monthly",
            "test",
            future_start + Duration::seconds(1),
        )
        .await
        .expect("new plan version becomes effective at the scheduled instant");
    assert_eq!(before_change.price_version_id, price_one.id);
    assert_eq!(after_change.price_version_id, price_two.id);
    assert_eq!(after_change.limit_value, Some(10));
}
