use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use sqlx::PgPool;
use uuid::Uuid;

use super::identity_access_acceptance_support::EventStreamCleanup;

pub struct TenantOutboxRecoveryContext<'a> {
    pub migration_pool: &'a PgPool,
    pub runtime_pool: &'a PgPool,
    pub jetstream: &'a jetstream::Context,
    pub failed_topology: &'a EventTopology,
    pub failed_namespace: &'a str,
    pub created_tenant_id: Uuid,
}

pub async fn verify(context: TenantOutboxRecoveryContext<'_>) {
    let TenantOutboxRecoveryContext {
        migration_pool,
        runtime_pool,
        jetstream,
        failed_topology,
        failed_namespace,
        created_tenant_id,
    } = context;
    // Exercise a deterministic JetStream failure without racing the
    // application's background relay: the synthetic Outbox row is leased
    // while the missing-stream publish is attempted.
    let failed_event_id = Uuid::new_v4();
    let failed_aggregate_id = Uuid::new_v4();
    let failed_lease_token = Uuid::new_v4();
    let failed_correlation_id = "corr_jetstream_publish_retry";
    let failed_payload = serde_json::json!({"tenant_id": created_tenant_id});
    sqlx::query(
        "INSERT INTO tenant_registry.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id,lease_token,lease_until,attempts) VALUES ($1,$2,1,'tenant.registered',1,'tenant_and_platform',$3,$4,$5,now()+interval '30 seconds',1)",
    )
    .bind(failed_event_id)
    .bind(failed_aggregate_id)
    .bind(&failed_payload)
    .bind(failed_correlation_id)
    .bind(failed_lease_token)
    .execute(&migration_pool)
    .await
    .expect("synthetic Outbox event is leased for deterministic failure injection");
    let failed_event = platform_tenant_registry::infrastructure::persistence::PendingOutboxEvent {
        event_id: failed_event_id,
        aggregate_id: failed_aggregate_id,
        aggregate_version: 1,
        event_type: "tenant.registered".to_owned(),
        schema_version: 1,
        audience: "tenant_and_platform".to_owned(),
        payload: failed_payload.clone(),
        traceparent: None,
        correlation_id: failed_correlation_id.to_owned(),
        lease_token: failed_lease_token,
    };
    let jetstream_error_code =
        platform_tenant_registry::infrastructure::outbox_relay::publish_event(
            &jetstream,
            &failed_event,
            &failed_topology.subject_prefix,
        )
        .await
        .expect_err("publishing without a matching stream fails retryably");
    assert!(matches!(
        jetstream_error_code,
        "jetstream_publish_unavailable" | "jetstream_ack_unavailable"
    ));
    assert!(
        platform_tenant_registry::infrastructure::persistence::reschedule_outbox_event(
            &runtime_pool,
            failed_event_id,
            failed_lease_token,
            jetstream_error_code,
        )
        .await
        .expect("failed JetStream event is rescheduled"),
        "the failed event lease is released for retry"
    );
    let deferred_event_state: (Option<String>, bool) = sqlx::query_as(
        "SELECT last_error_code,published_at IS NULL FROM tenant_registry.outbox_events WHERE event_id=$1",
    )
    .bind(failed_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("deferred JetStream event state is queryable");
    assert_eq!(
        deferred_event_state.0.as_deref(),
        Some(jetstream_error_code)
    );
    assert!(deferred_event_state.1);

    let failed_stream = failed_topology
        .ensure_stream(&jetstream)
        .await
        .expect("JetStream stream is restored for retry");
    let mut failed_stream_cleanup = EventStreamCleanup {
        jetstream: jetstream.clone(),
        namespace: failed_namespace,
        armed: true,
    };
    let retry_lease_token = Uuid::new_v4();
    sqlx::query(
        "UPDATE tenant_registry.outbox_events SET available_at=now(),lease_token=$2,lease_until=now()+interval '30 seconds' WHERE event_id=$1 AND published_at IS NULL",
    )
    .bind(failed_event_id)
    .bind(retry_lease_token)
    .execute(&migration_pool)
    .await
    .expect("deferred event is leased again after JetStream recovery");
    let retry_event = platform_tenant_registry::infrastructure::persistence::PendingOutboxEvent {
        lease_token: retry_lease_token,
        ..failed_event
    };
    platform_tenant_registry::infrastructure::outbox_relay::publish_event(
        &jetstream,
        &retry_event,
        &failed_topology.subject_prefix,
    )
    .await
    .expect("JetStream retry succeeds after stream recovery");
    assert!(
        platform_tenant_registry::infrastructure::persistence::mark_outbox_published(
            &runtime_pool,
            failed_event_id,
            retry_lease_token,
        )
        .await
        .expect("successful retry is acknowledged in PostgreSQL"),
        "successful JetStream retry marks the event published"
    );
    let retried_jetstream_state: (bool, Option<String>) = sqlx::query_as(
        "SELECT published_at IS NOT NULL,last_error_code FROM tenant_registry.outbox_events WHERE event_id=$1",
    )
    .bind(failed_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("successful JetStream retry state is queryable");
    assert!(retried_jetstream_state.0);
    assert!(retried_jetstream_state.1.is_none());
    drop(failed_stream);
    failed_stream_cleanup.cleanup().await;
}
