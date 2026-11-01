use super::*;
use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use reqwest::Client;
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

use super::{
    identity_access_acceptance_support::{EventStreamCleanup, is_stream_not_found},
    identity_outbox_recovery, outbox_database_failures, tenant_event_delivery,
    tenant_outbox_recovery,
};

pub struct EventFixture {
    pub jetstream: jetstream::Context,
    pub failed_topology: EventTopology,
    pub event_topology: EventTopology,
    pub stream: jetstream::stream::Stream,
    pub event_payload: Vec<u8>,
    pub registration_event_id: Uuid,
    pub registration_subject: String,
    pub event_stream_cleanup: EventStreamCleanup,
    pub isolated_stream_cleanup: EventStreamCleanup,
}

pub async fn create(
    migration_pool: &PgPool,
    runtime_pool: &PgPool,
    runtime_role: &str,
    client: &Client,
    http_addr: SocketAddr,
    session_cookie: &str,
    csrf_cookie: &str,
    created_tenant_id: Uuid,
    registered_user_id: Uuid,
    legal_identification_number: &str,
) -> EventFixture {
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
    outbox_database_failures::verify(outbox_database_failures::OutboxDatabaseFailureContext {
        migration_pool,
        runtime_pool: &runtime_pool,
        runtime_role: &runtime_role,
        jetstream: &jetstream,
        event_topology: &event_topology,
    })
    .await;
    tenant_outbox_recovery::verify(tenant_outbox_recovery::TenantOutboxRecoveryContext {
        migration_pool,
        runtime_pool: &runtime_pool,
        jetstream: &jetstream,
        failed_topology: &failed_topology,
        failed_namespace: &failed_namespace,
        created_tenant_id,
    })
    .await;
    identity_outbox_recovery::verify(migration_pool, &runtime_pool, &jetstream).await;
    let event_delivery =
        tenant_event_delivery::verify(tenant_event_delivery::TenantEventDeliveryContext {
            client,
            migration_pool,
            runtime_pool: &runtime_pool,
            jetstream: &jetstream,
            event_topology: &event_topology,
            event_namespace,
            http_addr,
            session_cookie,
            csrf_cookie,
            created_tenant_id,
            registered_user_id,
            legal_identification_number: &legal_identification_number,
        })
        .await;
    let stream = event_delivery.stream;
    let event_payload = event_delivery.event_payload;
    let registration_event_id = event_delivery.registration_event_id;
    let registration_subject = event_delivery.registration_subject;
    let event_stream_cleanup = event_delivery.event_stream_cleanup;
    let isolated_stream_cleanup = event_delivery.isolated_stream_cleanup;
    EventFixture {
        jetstream,
        failed_topology,
        event_topology,
        stream,
        event_payload,
        registration_event_id,
        registration_subject,
        event_stream_cleanup,
        isolated_stream_cleanup,
    }
}
