use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use crate::identity_access_acceptance_support::{EventStreamCleanup, is_stream_not_found};
use super::OutboxInfrastructure;
use uuid::Uuid;

pub(super) async fn prepare() -> OutboxInfrastructure {
let nats_client = async_nats::ConnectOptions::with_user_and_password(
    std::env::var("NATS_USER").expect("development NATS user is configured"),
    std::env::var("NATS_PASSWORD").expect("development NATS password is configured"),
)
.connect(std::env::var("PLATFORM_NATS_URL").expect("development JetStream URL is configured"))
.await
.expect("real development NATS accepts authenticated connection");
let jetstream = jetstream::new(nats_client);
let failed_namespace = format!("e2e_failure_{}", Uuid::new_v4().simple());
let failed_topology = EventTopology::from_namespace(Some(&failed_namespace))
    .expect("failure-path namespace is valid");
failed_topology
    .ensure_stream(&jetstream)
    .await
    .expect("failure-path stream is created in real JetStream");
let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    let _cleanup = EventStreamCleanup {
        jetstream: jetstream.clone(),
        namespace: failed_namespace.clone(),
        armed: true,
    };
    panic!("exercise E2E stream cleanup during test unwinding");
}));
assert!(failure.is_err(), "the cleanup failure path was exercised");
assert!(
    jetstream
        .get_stream(&failed_topology.stream_name)
        .await
        .is_err_and(is_stream_not_found),
    "panic cleanup removes only the stream created for that failed execution"
);
let event_namespace = format!("e2e_test_{}", Uuid::new_v4().simple());
let event_topology = EventTopology::from_namespace(Some(&event_namespace))
    .expect("test event namespace is valid");

    OutboxInfrastructure { jetstream, failed_namespace, failed_topology, event_namespace, event_topology }
}
