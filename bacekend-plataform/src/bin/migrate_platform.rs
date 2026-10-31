use std::{env, process::ExitCode, time::Duration};

use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;
use url::Url;

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

fn validate_database_targets(
    migration_database_url: &str,
    runtime_database_url: &str,
    runtime_role: &str,
) -> Result<(), &'static str> {
    let migration_url = Url::parse(migration_database_url)
        .map_err(|_| "platform_migration_database_url_invalid")?;
    let runtime_url =
        Url::parse(runtime_database_url).map_err(|_| "platform_database_url_invalid")?;
    if !matches!(migration_url.scheme(), "postgres" | "postgresql")
        || !matches!(runtime_url.scheme(), "postgres" | "postgresql")
        || migration_url.path() != runtime_url.path()
        || migration_url.username().is_empty()
        || runtime_url.username().is_empty()
        || migration_url.username() == runtime_url.username()
        || runtime_url.username() != runtime_role
        || !valid_role_identifier(runtime_role)
    {
        return Err("platform_migration_database_target_invalid");
    }
    Ok(())
}

fn valid_role_identifier(role: &str) -> bool {
    let mut characters = role.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && role.len() <= 63
}

async fn grant_runtime_privileges(
    pool: &sqlx::PgPool,
    runtime_role: &str,
) -> Result<(), &'static str> {
    if !valid_role_identifier(runtime_role) {
        return Err("platform_runtime_db_role_invalid");
    }
    let mut transaction = pool.begin().await.map_err(|error| {
        log_runtime_privilege_failure(&error);
        "platform_runtime_privileges_failed"
    })?;
    sqlx::query("SELECT set_config('payvia.runtime_db_role', $1, true)")
        .bind(runtime_role)
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            log_runtime_privilege_failure(&error);
            "platform_runtime_privileges_failed"
        })?;
    sqlx::query(
        "DO $$
         DECLARE role_name text := current_setting('payvia.runtime_db_role');
         BEGIN
             EXECUTE format('GRANT USAGE ON SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO %I', role_name);
             EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO %I', role_name);
             EXECUTE format('GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO %I', role_name);
             EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO %I', role_name);
             EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas GRANT USAGE, SELECT ON SEQUENCES TO %I', role_name);
         END $$",
    )
    .execute(&mut *transaction)
    .await
    .map_err(|error| {
        log_runtime_privilege_failure(&error);
        "platform_runtime_privileges_failed"
    })?;
    transaction.commit().await.map_err(|error| {
        log_runtime_privilege_failure(&error);
        "platform_runtime_privileges_failed"
    })
}

fn log_runtime_privilege_failure(error: &sqlx::Error) {
    tracing::error!(
        event.name = "database.runtime_privileges.failed",
        component = "platform_migrations",
        operation = "grant_runtime_privileges",
        dependency = "postgresql",
        sqlstate = error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref()
            .unwrap_or("grant_failed"),
        error_class = %error,
        error_category = "authorization",
        retryable = false
    );
}

#[cfg(test)]
mod tests {
    use super::{valid_role_identifier, validate_database_targets};

    #[test]
    fn migration_requires_a_distinct_runtime_role_and_matching_database() {
        assert_eq!(
            validate_database_targets(
                "postgres://migrator:secret@db.internal/payvia",
                "postgres://runtime:secret@proxy.internal/payvia",
                "runtime"
            ),
            Ok(())
        );
        assert_eq!(
            validate_database_targets(
                "postgres://runtime:secret@db.internal/payvia",
                "postgres://runtime:secret@db.internal/payvia",
                "runtime"
            ),
            Err("platform_migration_database_target_invalid")
        );
        assert_eq!(
            validate_database_targets(
                "postgres://migrator:secret@db.internal/platform",
                "postgres://runtime:secret@db.internal/payvia",
                "runtime"
            ),
            Err("platform_migration_database_target_invalid")
        );
    }

    #[test]
    fn runtime_role_name_is_a_safe_postgresql_identifier() {
        assert!(valid_role_identifier("platform_runtime"));
        assert!(!valid_role_identifier("runtime;DROP ROLE admin"));
        assert!(!valid_role_identifier(""));
    }
}
