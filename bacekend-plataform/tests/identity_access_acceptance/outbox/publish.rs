use super::OutboxInfrastructure;
use crate::identity_access_acceptance_context::AcceptanceContext;
use crate::identity_access_acceptance_support::EventStreamCleanup;
use uuid::Uuid;

pub(super) async fn verify(ctx: &mut AcceptanceContext, infrastructure: OutboxInfrastructure) {
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
.bind(ctx.state.created_tenant_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("onboarding event identifier is durable");
let published_count = platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
    &ctx.runtime_pool,
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
.fetch_one(&ctx.migration_pool)
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
.fetch_one(&ctx.migration_pool)
.await
.expect("outbox publication state can be verified");
assert!(
    outbox_published,
    "Outbox is marked only after JetStream ack"
);
    ctx.state.jetstream = Some(jetstream);
    ctx.state.event_topology = Some(event_topology);
    ctx.state.stream = Some(stream);
    ctx.state.event_stream_cleanup = Some(event_stream_cleanup);
    ctx.state.isolated_stream_cleanup = Some(isolated_stream_cleanup);
    ctx.state.registration_subject = registration_subject;
    ctx.state.registration_event_id = registration_event_id;
}
