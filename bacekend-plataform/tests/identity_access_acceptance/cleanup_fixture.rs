use super::*;
use sqlx::PgPool;

pub async fn shutdown(
    servers: server_fixture::ServerFixture,
    runtime_pool: PgPool,
    migration_pool: PgPool,
    mut database_cleanup: identity_access_acceptance_support::AcceptanceDatabaseCleanup,
    admin_pool: PgPool,
    provider: SdkTracerProvider,
) {
    servers.shutdown().await;
    runtime_pool.close().await;
    migration_pool.close().await;
    database_cleanup.cleanup().await;
    admin_pool.close().await;
    provider.shutdown().expect("trace provider shuts down");
}
