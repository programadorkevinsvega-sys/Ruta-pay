use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use sqlx::PgPool;

pub struct OutboxDatabaseFailureContext<'a> {
    pub migration_pool: &'a PgPool,
    pub runtime_pool: &'a PgPool,
    pub runtime_role: &'a str,
    pub jetstream: &'a jetstream::Context,
    pub event_topology: &'a EventTopology,
}

pub async fn verify(context: OutboxDatabaseFailureContext<'_>) {
    let OutboxDatabaseFailureContext {
        migration_pool,
        runtime_pool,
        runtime_role,
        jetstream,
        event_topology,
    } = context;
    // Simulate a transient PostgreSQL write failure in the Outbox relay.
    // The failed claim must not consume or mutate events; restoring privileges
    // lets the same pending events be processed by the next relay attempt.
    let pending_before_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM tenant_registry.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("pending Outbox state before PostgreSQL failure is queryable");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE ON tenant_registry.outbox_events FROM \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("isolated runtime role loses Outbox update privilege for failure injection");
    let postgres_relay_failure =
        platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
            &runtime_pool,
            &jetstream,
            &event_topology.subject_prefix,
        )
        .await;
    assert!(
        postgres_relay_failure.is_err(),
        "PostgreSQL permission failure is surfaced to the relay worker"
    );
    let pending_after_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM tenant_registry.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("pending Outbox state after PostgreSQL failure is queryable");
    assert_eq!(
        pending_after_database_failure, pending_before_database_failure,
        "failed PostgreSQL claim must not partially lease or consume events"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT UPDATE ON tenant_registry.outbox_events TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("isolated runtime role regains Outbox update privilege");

    let pending_identity_before_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM identity_access.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity Outbox state before PostgreSQL failure is queryable");
    assert!(pending_identity_before_database_failure.0 > 0);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE ON identity_access.outbox_events FROM \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("identity Outbox update privilege is revoked for failure injection");
    let identity_postgres_relay_failure =
        platform_identity_access::infrastructure::outbox_relay::relay_batch(
            &runtime_pool,
            &jetstream,
            &event_topology.subject_prefix,
        )
        .await;
    assert!(
        identity_postgres_relay_failure.is_err(),
        "identity relay surfaces a PostgreSQL claim failure"
    );
    let pending_identity_after_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM identity_access.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity Outbox state after PostgreSQL failure is queryable");
    assert_eq!(
        pending_identity_after_database_failure, pending_identity_before_database_failure,
        "identity Outbox PostgreSQL failure must not partially claim or consume events"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT UPDATE ON identity_access.outbox_events TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("identity Outbox update privilege is restored");
}
