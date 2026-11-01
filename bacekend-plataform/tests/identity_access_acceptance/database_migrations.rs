use sqlx::PgPool;

pub async fn migrate_and_verify_permission_seeds(migration_pool: &PgPool) {
    platform_identity_access::infrastructure::persistence::MIGRATOR
        .run(migration_pool)
        .await
        .expect("identity access migrations apply to real PostgreSQL");
    platform_tenant_registry::infrastructure::persistence::MIGRATOR
        .run(migration_pool)
        .await
        .expect("tenant registry migrations apply to real PostgreSQL");
    platform_realtime::infrastructure::persistence::MIGRATOR
        .run(migration_pool)
        .await
        .expect("realtime journal migration applies to real PostgreSQL");
    platform_catalog_pricing::infrastructure::persistence::MIGRATOR
        .run(migration_pool)
        .await
        .expect("catalog pricing migrations apply to real PostgreSQL");
    platform_entitlements_quotas::infrastructure::persistence::MIGRATOR
        .run(migration_pool)
        .await
        .expect("entitlements and quotas migrations apply to real PostgreSQL");
    let tenant_role_permissions: Vec<(String, String)> = sqlx::query_as(
            "SELECT role, permission FROM identity_access.tenant_role_permissions ORDER BY role, permission",
        )
        .fetch_all(migration_pool)
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
        .fetch_all(migration_pool)
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
}
