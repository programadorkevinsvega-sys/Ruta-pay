use super::{privileges::grant_runtime_privileges, validation::validate_database_targets};
use sqlx::postgres::PgPoolOptions;
use std::{env, time::Duration};

pub(super) async fn migrate() -> Result<(), &'static str> {
    let database_url = env::var("PLATFORM_MIGRATION_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("platform_migration_database_url_missing")?;
    let runtime_database_url = env::var("PLATFORM_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("platform_database_url_missing")?;
    let runtime_role = env::var("PLATFORM_RUNTIME_DB_ROLE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("platform_runtime_db_role_missing")?;
    validate_database_targets(&database_url, &runtime_database_url, &runtime_role)?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .map_err(|error| {
            tracing::error!(
                event.name = "database.migration.connection.failed",
                component = "platform_migrations",
                operation = "connect_database",
                dependency = "postgresql",
                sqlstate = error
                    .as_database_error()
                    .and_then(|database| database.code())
                    .as_deref()
                    .unwrap_or("connection_failed"),
                error_category = "connectivity",
                retryable = true
            );
            "postgresql_connection_failed"
        })?;
    platform_identity_access::infrastructure::persistence::MIGRATOR
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(
                event.name = "database.module_migration.failed",
                component = "identity_access",
                operation = "apply_migrations",
                dependency = "postgresql",
                error_class = %error,
                error_category = "persistence",
                retryable = false
            );
            "identity_access_migration_failed"
        })?;
    platform_tenant_registry::infrastructure::persistence::MIGRATOR
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(
                event.name = "database.module_migration.failed",
                component = "tenant_registry",
                operation = "apply_migrations",
                dependency = "postgresql",
                error_class = %error,
                error_category = "persistence",
                retryable = false
            );
            "tenant_registry_migration_failed"
        })?;
    platform_realtime::infrastructure::persistence::MIGRATOR
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(
                event.name = "database.module_migration.failed",
                component = "realtime",
                operation = "apply_migrations",
                dependency = "postgresql",
                error_class = %error,
                error_category = "persistence",
                retryable = false
            );
            "realtime_migration_failed"
        })?;
    platform_catalog_pricing::infrastructure::persistence::MIGRATOR
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(event.name="database.module_migration.failed", component="catalog_pricing", operation="apply_migrations", dependency="postgresql", error_class=%error, error_category="persistence", retryable=false);
            "catalog_pricing_migration_failed"
        })?;
    platform_entitlements_quotas::infrastructure::persistence::MIGRATOR
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(event.name="database.module_migration.failed", component="entitlements_quotas", operation="apply_migrations", dependency="postgresql", error_class=%error, error_category="persistence", retryable=false);
            "entitlements_quotas_migration_failed"
        })?;
    grant_runtime_privileges(&pool, &runtime_role).await?;
    pool.close().await;
    tracing::info!(
        event.name = "database.migrations.applied",
        component = "platform_migrations",
        operation = "apply_coordinated_migrations",
        result = "success"
    );
    Ok(())
}

