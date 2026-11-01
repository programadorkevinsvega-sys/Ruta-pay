use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;
use crate::identity_access_acceptance_support::AcceptanceDatabaseCleanup;

pub(super) struct DatabaseState {
    pub(super) admin_pool: sqlx::PgPool,
    pub(super) runtime_role: String,
    pub(super) database_cleanup: AcceptanceDatabaseCleanup,
    pub(super) migration_pool: sqlx::PgPool,
    pub(super) runtime_pool: sqlx::PgPool,
    pub(super) redis_url: String,
}

pub(super) async fn prepare() -> DatabaseState {
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
platform_identity_access::infrastructure::persistence::MIGRATOR
    .run(&migration_pool)
    .await
    .expect("identity access migrations apply to real PostgreSQL");
platform_tenant_registry::infrastructure::persistence::MIGRATOR
    .run(&migration_pool)
    .await
    .expect("tenant registry migrations apply to real PostgreSQL");
platform_realtime::infrastructure::persistence::MIGRATOR
    .run(&migration_pool)
    .await
    .expect("realtime journal migration applies to real PostgreSQL");
platform_catalog_pricing::infrastructure::persistence::MIGRATOR
    .run(&migration_pool)
    .await
    .expect("catalog pricing migrations apply to real PostgreSQL");
platform_entitlements_quotas::infrastructure::persistence::MIGRATOR
    .run(&migration_pool)
    .await
    .expect("entitlements and quotas migrations apply to real PostgreSQL");
let tenant_role_permissions: Vec<(String, String)> = sqlx::query_as(
    "SELECT role, permission FROM identity_access.tenant_role_permissions ORDER BY role, permission",
)
.fetch_all(&migration_pool)
.await
.expect("tenant role permission matrix is queryable");
assert_eq!(
    tenant_role_permissions,
    vec![
        ("developer".to_owned(), "api_keys.manage".to_owned()),
        ("developer".to_owned(), "tenant.read".to_owned()),
        ("developer".to_owned(), "usage.consume".to_owned()),
        ("developer".to_owned(), "usage.read".to_owned()),
        ("tenant_admin".to_owned(), "api_keys.manage".to_owned()),
        ("tenant_admin".to_owned(), "members.manage".to_owned()),
        ("tenant_admin".to_owned(), "tenant.read".to_owned()),
        (
            "tenant_admin".to_owned(),
            "tenant.settings.manage".to_owned()
        ),
        ("tenant_admin".to_owned(), "usage.consume".to_owned()),
        ("tenant_admin".to_owned(), "usage.read".to_owned()),
        ("tenant_owner".to_owned(), "api_keys.manage".to_owned()),
        ("tenant_owner".to_owned(), "members.manage".to_owned()),
        ("tenant_owner".to_owned(), "tenant.read".to_owned()),
        (
            "tenant_owner".to_owned(),
            "tenant.settings.manage".to_owned()
        ),
        ("tenant_owner".to_owned(), "usage.consume".to_owned()),
        ("tenant_owner".to_owned(), "usage.read".to_owned()),
        ("viewer".to_owned(), "tenant.read".to_owned()),
        ("viewer".to_owned(), "usage.read".to_owned()),
    ],
    "tenant role-to-permission grants must remain explicit and least-privilege"
);
let operator_role_permissions: Vec<(String, String)> = sqlx::query_as(
    "SELECT role, permission FROM identity_access.operator_role_permissions ORDER BY role, permission",
)
.fetch_all(&migration_pool)
.await
.expect("operator role permission matrix is queryable");
assert_eq!(
    operator_role_permissions,
    vec![
        ("platform_owner".to_owned(), "catalog.manage".to_owned()),
        (
            "platform_owner".to_owned(),
            "operators.bootstrap".to_owned()
        ),
        ("platform_owner".to_owned(), "tenants.activate".to_owned()),
        ("platform_owner".to_owned(), "tenants.close".to_owned()),
        ("platform_owner".to_owned(), "tenants.read".to_owned()),
        ("platform_owner".to_owned(), "tenants.suspend".to_owned()),
        ("platform_owner".to_owned(), "tenants.verify".to_owned()),
        ("platform_security".to_owned(), "catalog.manage".to_owned()),
        (
            "platform_security".to_owned(),
            "tenants.activate".to_owned()
        ),
        ("platform_security".to_owned(), "tenants.close".to_owned()),
        ("platform_security".to_owned(), "tenants.read".to_owned()),
        ("platform_security".to_owned(), "tenants.suspend".to_owned()),
        ("platform_security".to_owned(), "tenants.verify".to_owned()),
        ("platform_support".to_owned(), "tenants.read".to_owned()),
    ],
    "operator role-to-permission grants must remain explicit and least-privilege"
);
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
.execute(&migration_pool)
.await
.expect("runtime role receives schema usage");
sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\"")))
    .execute(&migration_pool)
    .await
    .expect("runtime role receives owned-module table privileges");
sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\"")))
    .execute(&migration_pool)
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

    DatabaseState {
        admin_pool,
        runtime_role,
        database_cleanup,
        migration_pool,
        runtime_pool,
        redis_url,
    }
}
