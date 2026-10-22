use std::{env, process::ExitCode, time::Duration};

use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    match migrate().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => {
            tracing::error!(
                event.name = "database.migration.failed",
                component = "platform_migrations",
                operation = "apply_coordinated_migrations",
                dependency = "postgresql",
                error_code = code,
                error_category = "persistence",
                retryable = false
            );
            ExitCode::FAILURE
        }
    }
}

async fn migrate() -> Result<(), &'static str> {
    let database_url = env::var("PLATFORM_MIGRATION_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("platform_migration_database_url_missing")?;
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
    pool.close().await;
    tracing::info!(
        event.name = "database.migrations.applied",
        component = "platform_migrations",
        operation = "apply_coordinated_migrations",
        result = "success"
    );
    Ok(())
}
