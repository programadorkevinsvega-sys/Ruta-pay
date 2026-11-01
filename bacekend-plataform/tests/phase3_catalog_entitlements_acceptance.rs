use chrono::{Duration, Utc};
use platform_catalog_pricing::application::catalog_service::{
    CatalogService, PublishPriceVersionCommand,
};
use platform_entitlements_quotas::{
    application::entitlement_service::{
        ConsumeQuotaCommand, EntitlementService, EntitlementServiceError,
    },
    domain::entitlement::{Capability, EntitlementError},
};
use sqlx::postgres::PgPoolOptions;
use std::{str::FromStr, time::Duration as StdDuration};
use uuid::Uuid;

struct AcceptanceDatabaseCleanup {
    admin_pool: sqlx::PgPool,
    database_name: String,
    runtime_role: String,
    armed: bool,
}

impl AcceptanceDatabaseCleanup {
    async fn cleanup(&mut self) {
        Self::drop_resources(&self.admin_pool, &self.database_name, &self.runtime_role)
            .await
            .expect("phase three isolated database and role are removed");
        self.armed = false;
    }

    async fn drop_resources(
        admin_pool: &sqlx::PgPool,
        database_name: &str,
        runtime_role: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=$1 AND pid<>pg_backend_pid()")
            .bind(database_name).execute(admin_pool).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{database_name}\""
        )))
        .execute(admin_pool)
        .await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP ROLE IF EXISTS \"{runtime_role}\""
        )))
        .execute(admin_pool)
        .await?;
        Ok(())
    }
}

impl Drop for AcceptanceDatabaseCleanup {
    fn drop(&mut self) {
        if self.armed {
            let pool = self.admin_pool.clone();
            let database = self.database_name.clone();
            let role = self.runtime_role.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                runtime.block_on(async move {
                    Self::drop_resources(&pool, &database, &role).await.expect(
                        "panic cleanup removes only this isolated phase three database and role",
                    );
                })
            });
        }
    }
}

fn capability(key: &str, limit: Option<i64>, environment: &str, overage: &str) -> Capability {
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

fn is_domain_error(error: EntitlementServiceError, expected: EntitlementError) -> bool {
    matches!(error, EntitlementServiceError::Domain(actual) if actual == expected)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phase_three_real_postgres_limits_replay_concurrency_versioning_and_rls() {
    let admin_url = std::env::var("PLATFORM_TEST_DATABASE_ADMIN_URL")
        .expect("CI provides an administrator PostgreSQL URL");
    let admin_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&admin_url)
        .await
        .expect("administrator PostgreSQL connection succeeds");
    let suffix = Uuid::new_v4().simple().to_string();
    let database_name = format!("phase3_{suffix}");
    let runtime_role = format!("phase3_{suffix}");
    let runtime_password = Uuid::new_v4().simple().to_string();

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE \"{database_name}\""
    )))
    .execute(&admin_pool)
    .await
    .expect("isolated phase three database is created");
    let mut cleanup = AcceptanceDatabaseCleanup {
        admin_pool: admin_pool.clone(),
        database_name: database_name.clone(),
        runtime_role: runtime_role.clone(),
        armed: true,
    };

    let mut database_url = url::Url::from_str(&admin_url).expect("administrator URL parses");
    database_url.set_path(&format!("/{database_name}"));
    let migration_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(database_url.as_str())
        .await
        .expect("isolated database accepts migration connection");

    platform_identity_access::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("identity migrations apply");
    platform_tenant_registry::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("tenant migrations apply");
    platform_realtime::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("realtime migrations apply");
    platform_catalog_pricing::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("catalog migrations apply");
    platform_entitlements_quotas::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("entitlement migrations apply");

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE \"{runtime_role}\" LOGIN PASSWORD '{runtime_password}'"
    )))
    .execute(&admin_pool)
    .await
    .expect("restricted application role is created");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT CONNECT ON DATABASE \"{database_name}\" TO \"{runtime_role}\""
    )))
    .execute(&admin_pool)
    .await
    .expect("runtime role receives database connect");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA identity_access,tenant_registry,realtime,catalog_pricing,entitlements_quotas TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("runtime role receives schema usage");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA identity_access,tenant_registry,realtime,catalog_pricing,entitlements_quotas TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("runtime role receives table privileges");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA identity_access,tenant_registry,realtime,catalog_pricing,entitlements_quotas TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("runtime role receives sequence privileges");

    database_url
        .set_username(&runtime_role)
        .expect("runtime role username is valid");
    database_url
        .set_password(Some(&runtime_password))
        .expect("runtime role password is valid");
    let runtime_pool = PgPoolOptions::new()
        .max_connections(12)
        .acquire_timeout(StdDuration::from_secs(5))
        .connect(database_url.as_str())
        .await
        .expect("restricted runtime role connects");
    let bypass_rls: bool =
        sqlx::query_scalar("SELECT rolbypassrls FROM pg_roles WHERE rolname=current_user")
            .fetch_one(&runtime_pool)
            .await
            .expect("runtime role RLS attribute is readable");
    assert!(!bypass_rls, "application role must not bypass RLS");

    let catalog = CatalogService::new(runtime_pool.clone());
    let entitlements = EntitlementService::new(runtime_pool.clone());
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
        .assign_plan(
            actor,
            tenant_one,
            price_one.id,
            price_start,
            None,
            correlation,
        )
        .await
        .expect("first tenant receives the plan");
    entitlements
        .assign_plan(
            actor,
            tenant_two,
            price_one.id,
            price_start,
            None,
            correlation,
        )
        .await
        .expect("second tenant receives the same plan");

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

    runtime_pool.close().await;
    migration_pool.close().await;
    cleanup.cleanup().await;
}
