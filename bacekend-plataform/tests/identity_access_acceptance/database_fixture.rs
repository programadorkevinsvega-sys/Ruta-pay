use super::*;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use uuid::Uuid;

use super::identity_access_acceptance_support::AcceptanceDatabaseCleanup;

pub struct IdentityDatabaseFixture {
    pub redis_url: String,
    pub runtime_role: String,
    pub admin_pool: PgPool,
    pub migration_pool: PgPool,
    pub runtime_pool: PgPool,
    pub cleanup: AcceptanceDatabaseCleanup,
}

pub async fn create() -> IdentityDatabaseFixture {
    let admin_url = std::env::var("PLATFORM_TEST_DATABASE_ADMIN_URL")
        .expect("development/admin PostgreSQL URL is configured for integration acceptance");
    let redis_url =
        std::env::var("PLATFORM_REDIS_URL").expect("real development Redis URL is configured");
    let admin_pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&admin_url)
        .await
        .expect("development PostgreSQL accepts the admin connection");
    let database_name = format!("phase2_{}", Uuid::new_v4().simple());
    let runtime_role = format!("phase2_{}", Uuid::new_v4().simple());
    let runtime_password = Uuid::new_v4().simple().to_string();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE \"{database_name}\""
    )))
    .execute(&admin_pool)
    .await
    .expect("isolated acceptance database is created");
    let mut database_cleanup = AcceptanceDatabaseCleanup {
        admin_pool: admin_pool.clone(),
        database_name: database_name.clone(),
        runtime_role: runtime_role.clone(),
        armed: true,
    };

    let mut parsed_url = url::Url::parse(&admin_url).expect("admin database URL parses");
    parsed_url.set_path(&format!("/{database_name}"));
    let migration_pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(parsed_url.as_str())
        .await
        .expect("isolated acceptance database accepts migration connection");
    database_migrations::migrate_and_verify_permission_seeds(migration_pool).await;

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE \"{runtime_role}\" LOGIN PASSWORD '{runtime_password}'"
    )))
    .execute(&admin_pool)
    .await
    .expect("a restricted application role is created");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT CONNECT ON DATABASE \"{database_name}\" TO \"{runtime_role}\""
    )))
    .execute(&admin_pool)
    .await
    .expect("runtime role receives database connect only");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\""
        )))
        .execute(migration_pool)
        .await
        .expect("runtime role receives schema usage");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\"")))
            .execute(migration_pool)
            .await
            .expect("runtime role receives owned-module table privileges");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\"")))
            .execute(migration_pool)
            .await
            .expect("runtime role receives sequence privileges for every migrated module");
    parsed_url
        .set_username(&runtime_role)
        .expect("runtime role is valid in database URL");
    parsed_url
        .set_password(Some(&runtime_password))
        .expect("runtime password is valid in database URL");
    let runtime_pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(3))
        .connect(parsed_url.as_str())
        .await
        .expect("non-superuser runtime role connects to the isolated database");
    let bypass_rls: bool =
        sqlx::query_scalar("SELECT rolbypassrls FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&runtime_pool)
            .await
            .expect("runtime RLS attribute is readable");
    assert!(
        !bypass_rls,
        "runtime identity must not bypass row-level security"
    );
    IdentityDatabaseFixture {
        redis_url,
        runtime_role,
        admin_pool,
        migration_pool,
        runtime_pool,
        cleanup: database_cleanup,
    }
}
