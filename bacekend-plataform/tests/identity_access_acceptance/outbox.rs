mod setup;
mod database_failures;
mod tenant_retry;
mod identity_retry;

use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use uuid::Uuid;
use sqlx::PgPool;
use crate::identity_access_acceptance_support::EventStreamCleanup;
use crate::identity_access_acceptance_support::EventStreamCleanup;

pub(super) struct OutboxInfrastructure {
    pub(super) jetstream: jetstream::Context,
    pub(super) failed_namespace: String,
    pub(super) failed_topology: EventTopology,
    pub(super) event_namespace: String,
    pub(super) event_topology: EventTopology,
}

pub(super) struct OutboxState {
    pub(super) jetstream: jetstream::Context,
    pub(super) event_topology: EventTopology,
    pub(super) stream: jetstream::stream::Stream,
    pub(super) event_stream_cleanup: EventStreamCleanup,
    pub(super) isolated_stream_cleanup: EventStreamCleanup,
    pub(super) registration_subject: String,
    pub(super) registration_event_id: Uuid,
}

pub(super) async fn run(
    migration_pool: &sqlx::PgPool,
    runtime_pool: &sqlx::PgPool,
    runtime_role: &str,
    created_tenant_id: Uuid,
) -> OutboxState {
    let infrastructure = setup::prepare().await;
    database_failures::verify(migration_pool, runtime_pool, runtime_role, &infrastructure).await;
    tenant_retry::verify(migration_pool, runtime_pool, created_tenant_id, &infrastructure).await;
    identity_retry::verify(migration_pool, runtime_pool, &infrastructure).await;
    publish(infrastructure, migration_pool, runtime_pool, created_tenant_id).await
}

pub(super) async fn publish(
    infrastructure: OutboxInfrastructure,
    migration_pool: &PgPool,
    runtime_pool: &PgPool,
    created_tenant_id: Uuid,
) -> OutboxState {
    let OutboxInfrastructure { jetstream, event_topology, event_namespace, .. } = infrastructure;
let stream = event_topology
    .ensure_stream(&jetstream)
    .await
    .expect("isolated real JetStream stream is available");
let event_stream_cleanup = EventStreamCleanup {
    jetstream: jetstream.clone(),
    namespace: event_namespace,
    armed: true,
};
let isolated_namespace = format!("e2e_isolation_{}", Uuid::new_v4().simple());
let isolated_topology = EventTopology::from_namespace(Some(&isolated_namespace))
    .expect("second test namespace is valid");
isolated_topology
    .ensure_stream(&jetstream)
    .await
    .expect("second isolated JetStream stream is available");
let isolated_stream_cleanup = EventStreamCleanup {
    jetstream: jetstream.clone(),
    namespace: isolated_namespace,
    armed: true,
};
let registration_subject = format!("{}.tenant.registered", event_topology.subject_prefix);
let registration_event_id: Uuid = sqlx::query_scalar(
    "SELECT event_id FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
)
.bind(created_tenant_id)
.fetch_one(&migration_pool)
.await
.expect("onboarding event identifier is durable");
let published_count = platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
    &runtime_pool,
    &jetstream,
    &event_topology.subject_prefix,
)
.await
.expect("tenant outbox relay publishes through the real JetStream client");
assert!(
    published_count > 0,
    "outbox events are acknowledged durably after dependency recovery"
);
let retried_event_state: (bool, Option<String>) = sqlx::query_as(
    "SELECT published_at IS NOT NULL,last_error_code FROM tenant_registry.outbox_events WHERE event_id=$1",
)
.bind(registration_event_id)
.fetch_one(&migration_pool)
.await
.expect("retried Outbox event state is queryable");
assert!(retried_event_state.0);
assert!(retried_event_state.1.is_none());
let isolated_stream = jetstream
    .get_stream(&isolated_topology.stream_name)
    .await
    .expect("second isolated stream remains available");
let isolated_subject = format!("{}.tenant.registered", isolated_topology.subject_prefix);
let isolated_read = isolated_stream
    .get_last_raw_message_by_subject(&isolated_subject)
    .await;
assert!(
    isolated_read.is_err_and(|error| {
        error.kind() == async_nats::jetstream::stream::LastRawMessageErrorKind::NoMessageFound
    }),
    "an event published to one E2E namespace is absent from another"
);
let outbox_published: bool = sqlx::query_scalar(
    "SELECT published_at IS NOT NULL FROM tenant_registry.outbox_events WHERE event_id=$1",
)
.bind(registration_event_id)
.fetch_one(&migration_pool)
.await
.expect("outbox publication state can be verified");
assert!(
    outbox_published,
    "Outbox is marked only after JetStream ack"
);
    OutboxState {
        jetstream,
        event_topology,
        stream,
        event_stream_cleanup,
        isolated_stream_cleanup,
        registration_subject,
        registration_event_id,
    }
}
