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


pub struct TestDatabase {
    pub migration_pool: sqlx::PgPool,
    pub runtime_pool: sqlx::PgPool,
    cleanup: AcceptanceDatabaseCleanup,
}

impl TestDatabase {
    pub async fn new() -> Self {
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
        let cleanup = AcceptanceDatabaseCleanup {
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
    
        Self { migration_pool, runtime_pool, cleanup }
    }

    pub async fn finish(self) {
        self.runtime_pool.close().await;
        self.migration_pool.close().await;
        let mut cleanup = self.cleanup;
        cleanup.cleanup().await;
    }
}
