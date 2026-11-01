use chrono::{Duration, Utc};
use platform_catalog_pricing::application::catalog_service::{
    CatalogService, PublishPriceVersionCommand,
};
use platform_entitlements_quotas::{
    application::entitlement_service::EntitlementService,
    domain::entitlement::{Capability, EntitlementError},
};
use uuid::Uuid;

use super::database::TestDatabase;

pub struct TestFixture {
    pub database: TestDatabase,
    pub catalog: CatalogService,
    pub entitlements: EntitlementService,
    pub actor: Uuid,
    pub tenant_one: Uuid,
    pub tenant_two: Uuid,
    pub tenant_unassigned: Uuid,
    pub now: chrono::DateTime<Utc>,
    pub future_start: chrono::DateTime<Utc>,
    pub plan: platform_catalog_pricing::domain::catalog::Plan,
    pub price_one: platform_catalog_pricing::domain::catalog::PriceVersion,
    pub correlation: &'static str,
}

impl TestFixture {
    pub async fn new() -> Self {
        let database = TestDatabase::new().await;
        let catalog = CatalogService::new(database.runtime_pool.clone());
        let entitlements = EntitlementService::new(database.runtime_pool.clone());
        let actor = Uuid::new_v4();
        let tenant_one = Uuid::new_v4();
        let tenant_two = Uuid::new_v4();
        let tenant_unassigned = Uuid::new_v4();
        let now = Utc::now();
        let price_start = now - Duration::days(2);
        let future_start = now + Duration::days(20);
        let correlation = "phase3_real_postgres_acceptance";

        let plan = catalog
            .create_plan(
                actor,
                "acceptance-plan",
                "Acceptance Plan",
                "real database acceptance",
                correlation,
            )
            .await
            .expect("plan creation persists through the application service");
        let price_one = catalog
            .publish_price_version(PublishPriceVersionCommand {
                actor_id: actor,
                plan_id: plan.id,
                currency: "USD",
                amount_minor: 1200,
                period_unit: "month",
                interval_count: 1,
                effective_from: price_start,
                effective_until: Some(future_start),
                correlation_id: correlation,
            })
            .await
            .expect("first immutable price version is published");
        let capabilities_one = vec![
            capability("payments.monthly", Some(5), "all", "deny"),
            capability("reports.export", Some(1), "all", "allow"),
            capability("live.feature", Some(10), "live", "deny"),
        ];
        entitlements
            .set_capabilities(actor, price_one.id, &capabilities_one, correlation)
            .await
            .expect("capability snapshot is persisted");
        entitlements
            .assign_plan(actor, tenant_one, price_one.id, price_start, None, correlation)
            .await
            .expect("first tenant receives the plan");
        entitlements
            .assign_plan(actor, tenant_two, price_one.id, price_start, None, correlation)
            .await
            .expect("second tenant receives the same plan");

        Self {
            database,
            catalog,
            entitlements,
            actor,
            tenant_one,
            tenant_two,
            tenant_unassigned,
            now,
            future_start,
            plan,
            price_one,
            correlation,
        }
    }

    pub async fn finish(self) {
        self.database.finish().await;
    }
}

pub(super) fn capability(
    key: &str,
    limit: Option<i64>,
    environment: &str,
    overage: &str,
) -> Capability {
    Capability {
        key: key.to_owned(),
        enabled: true,
        limit_value: limit,
        unit: "operations".to_owned(),
        overage_policy: overage.to_owned(),
        environment: environment.to_owned(),
        quota_period: "month".to_owned(),
    }
}

pub(super) fn is_domain_error(
    error: platform_entitlements_quotas::application::entitlement_service::EntitlementServiceError,
    expected: EntitlementError,
) -> bool {
    matches!(error, platform_entitlements_quotas::application::entitlement_service::EntitlementServiceError::Domain(actual) if actual == expected)
}
