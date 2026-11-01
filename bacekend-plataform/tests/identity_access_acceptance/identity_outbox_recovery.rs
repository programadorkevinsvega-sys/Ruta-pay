use super::*;
use async_nats::jetstream;
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn verify(
    migration_pool: &PgPool,
    runtime_pool: &PgPool,
    jetstream: &jetstream::Context,
) {
    // Exercise the identity_access Outbox worker against a real JetStream
    // publish failure and recovery, independently of tenant_registry.
    sqlx::query(
        "UPDATE identity_access.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL",
    )
    .execute(migration_pool)
    .await
    .expect("existing identity Outbox events are deferred for isolated retry acceptance");
    let identity_failure_namespace = format!("e2e_identity_{}", Uuid::new_v4().simple());
    let identity_failure_topology =
        EventTopology::from_namespace(Some(&identity_failure_namespace))
            .expect("identity failure namespace is valid");
    let identity_failed_event_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity_access.outbox_events          (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id)          VALUES ($1,$2,1,'identity.session.revoked',1,'platform_operator',$3,'corr_identity_jetstream_retry')",
    )
    .bind(identity_failed_event_id)
    .bind(Uuid::new_v4())
    .bind(serde_json::json!({"session_id": Uuid::new_v4()}))
    .execute(migration_pool)
    .await
    .expect("identity Outbox retry event is inserted");
    let failed_identity_delivery =
        platform_identity_access::infrastructure::outbox_relay::relay_batch(
            runtime_pool,
            &jetstream,
            &identity_failure_topology.subject_prefix,
        )
        .await
        .expect("identity relay classifies and reschedules JetStream publish failure");
    assert_eq!(failed_identity_delivery, 0);
    let identity_deferred_state: (Option<String>, bool, bool) = sqlx::query_as(
        "SELECT last_error_code,published_at IS NULL,lease_token IS NULL FROM identity_access.outbox_events WHERE event_id=$1",
    )
    .bind(identity_failed_event_id)
    .fetch_one(migration_pool)
    .await
    .expect("identity deferred Outbox state is queryable");
    assert_eq!(
        identity_deferred_state.0.as_deref(),
        Some("jetstream_ack_unavailable")
    );
    assert!(identity_deferred_state.1);
    assert!(identity_deferred_state.2);
    identity_failure_topology
        .ensure_stream(&jetstream)
        .await
        .expect("identity JetStream stream is restored for retry");
    sqlx::query("UPDATE identity_access.outbox_events SET available_at=now() WHERE event_id=$1")
        .bind(identity_failed_event_id)
        .execute(migration_pool)
        .await
        .expect("identity failed event becomes eligible after dependency recovery");
    let identity_retried_delivery =
        platform_identity_access::infrastructure::outbox_relay::relay_batch(
            runtime_pool,
            &jetstream,
            &identity_failure_topology.subject_prefix,
        )
        .await
        .expect("identity relay retries the durable event after JetStream recovery");
    assert_eq!(identity_retried_delivery, 1);
    let identity_retried_state: (bool, Option<String>) = sqlx::query_as(
        "SELECT published_at IS NOT NULL,last_error_code FROM identity_access.outbox_events WHERE event_id=$1",
    )
    .bind(identity_failed_event_id)
    .fetch_one(migration_pool)
    .await
    .expect("identity successful retry state is queryable");
    assert!(identity_retried_state.0);
    assert!(identity_retried_state.1.is_none());
    delete_e2e_stream(&jetstream, &identity_failure_namespace)
        .await
        .expect("identity failure stream cleanup removes only its isolated stream");
}
