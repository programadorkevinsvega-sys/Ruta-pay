use super::validation::valid_role_identifier;

pub(super) async fn grant_runtime_privileges(
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
