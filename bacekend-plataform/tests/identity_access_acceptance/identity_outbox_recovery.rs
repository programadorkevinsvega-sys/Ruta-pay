use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use sqlx::PgPool;
use uuid::Uuid;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub async fn verify(
    migration_pool: &PgPool,
    runtime_pool: &PgPool,
    recovery_jetstream: &jetstream::Context,
    failed_topology: &EventTopology,
    recovery_topology: &EventTopology,
    captured_events: &Arc<Mutex<Vec<HashMap<String, String>>>>,
) {
    sqlx::query(
        "UPDATE identity_access.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL",
    )
    .execute(migration_pool)
    .await
    .expect("existing identity Outbox events are deferred for isolated retry acceptance");
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
            recovery_jetstream,
            &failed_topology.subject_prefix,
        )
        .await
        .expect("identity relay classifies and reschedules JetStream publish failure");
    assert_eq!(failed_identity_delivery, 0);
    let identity_failure_diagnostic = captured_events
        .lock()
        .expect("event capture lock")
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("outbox.jetstream.publish.failed"))
                && event
                    .get("event_id")
                    .is_some_and(|event_id| event_id.contains(&identity_failed_event_id.to_string()))
        })
        .cloned()
        .expect("identity JetStream failure diagnostic includes the failed event");
    assert!(
        identity_failure_diagnostic
            .get("error_code")
            .is_some_and(|code| {
                matches!(
                    code.as_str(),
                    "jetstream_publish_unavailable" | "jetstream_ack_unavailable"
                )
            })
    );
    assert!(
        identity_failure_diagnostic
            .get("subject")
            .is_some_and(|subject| subject.contains(&failed_topology.subject_prefix))
    );
    let identity_deferred_state: (Option<String>, bool, bool) = sqlx::query_as(
        "SELECT last_error_code,published_at IS NULL,lease_token IS NULL FROM identity_access.outbox_events WHERE event_id=$1",
    )
    .bind(identity_failed_event_id)
    .fetch_one(migration_pool)
    .await
    .expect("identity deferred Outbox state is queryable");
    assert_eq!(
        identity_deferred_state.0.as_deref(),
        identity_failure_diagnostic
            .get("error_code")
            .map(String::as_str)
    );
    assert!(identity_deferred_state.1);
    assert!(identity_deferred_state.2);
    sqlx::query("UPDATE identity_access.outbox_events SET available_at=now() WHERE event_id=$1")
        .bind(identity_failed_event_id)
        .execute(migration_pool)
        .await
        .expect("identity failed event becomes eligible after dependency recovery");
    let identity_retried_delivery =
        platform_identity_access::infrastructure::outbox_relay::relay_batch(
            runtime_pool,
            recovery_jetstream,
            &recovery_topology.subject_prefix,
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
}
