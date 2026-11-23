use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use reqwest::Client;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

use super::{
    identity_access_acceptance_support::{EventStreamCleanup, is_stream_not_found},
    identity_outbox_recovery, outbox_database_failures, tenant_event_delivery,
    tenant_outbox_recovery,
};

pub struct EventFixture {
    pub jetstream: jetstream::Context,
    pub e2e_jetstream: jetstream::Context,
    pub event_topology: EventTopology,
    pub stream: jetstream::stream::Stream,
    pub event_payload: Vec<u8>,
    pub registration_event_id: Uuid,
    pub registration_subject: String,
    pub event_stream_cleanup: EventStreamCleanup,
    pub isolated_stream_cleanup: EventStreamCleanup,
}

pub struct EventFixtureContext<'a> {
    pub migration_pool: &'a PgPool,
    pub runtime_pool: &'a PgPool,
    pub runtime_role: &'a str,
    pub client: &'a Client,
    pub http_addr: SocketAddr,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
    pub registered_user_id: Uuid,
    pub legal_identification_number: &'a str,
    pub captured_events: &'a Arc<Mutex<Vec<HashMap<String, String>>>>,
}

pub async fn create(context: EventFixtureContext<'_>) -> EventFixture {
    let EventFixtureContext {
        migration_pool, runtime_pool, runtime_role, client, http_addr,
        session_cookie, csrf_cookie, created_tenant_id, registered_user_id,
        legal_identification_number,
        captured_events,
    } = context;
    let nats_client = async_nats::ConnectOptions::with_user_and_password(
        std::env::var("NATS_USER").expect("development NATS user is configured"),
        std::env::var("NATS_PASSWORD").expect("development NATS password is configured"),
    )
    .connect(std::env::var("PLATFORM_NATS_URL").expect("development JetStream URL is configured"))
    .await
    .expect("real development NATS accepts authenticated connection");
    let jetstream = jetstream::new(nats_client);
    let e2e_nats_client = async_nats::ConnectOptions::with_user_and_password(
        std::env::var("PLATFORM_TEST_NATS_E2E_USER")
            .expect("dedicated NATS E2E test user is configured"),
        std::env::var("PLATFORM_TEST_NATS_E2E_PASSWORD")
            .expect("dedicated NATS E2E test password is configured"),
    )
    .connect(std::env::var("PLATFORM_NATS_URL").expect("development JetStream URL is configured"))
    .await
    .expect("dedicated NATS E2E identity connects");
    let e2e_jetstream = jetstream::new(e2e_nats_client);
    let recovery_nats_client = async_nats::ConnectOptions::with_user_and_password(
        std::env::var("PLATFORM_TEST_NATS_RECOVERY_USER")
            .expect("dedicated NATS recovery test user is configured"),
        std::env::var("PLATFORM_TEST_NATS_RECOVERY_PASSWORD")
            .expect("dedicated NATS recovery test password is configured"),
    )
    .event_callback(|event| async move {
        match event {
            async_nats::Event::ServerError(
                async_nats::ServerError::AuthorizationViolation,
            ) => tracing::error!(
                event.name = "nats.server.authorization_violation",
                component = "tenant_registry",
                operation = "publish_outbox_event",
                dependency = "nats_jetstream",
                error_code = "nats_authorization_violation",
                error_category = "authorization",
                retryable = false,
            ),
            async_nats::Event::ServerError(_) => tracing::error!(
                event.name = "nats.server.error",
                component = "tenant_registry",
                operation = "publish_outbox_event",
                dependency = "nats_jetstream",
                error_code = "nats_server_error",
                error_category = "dependency",
            ),
            async_nats::Event::Disconnected => tracing::warn!(
                event.name = "nats.connection.disconnected",
                component = "tenant_registry",
                operation = "publish_outbox_event",
                dependency = "nats_jetstream",
            ),
            async_nats::Event::Connected => tracing::info!(
                event.name = "nats.connection.connected",
                component = "tenant_registry",
                operation = "publish_outbox_event",
                dependency = "nats_jetstream",
            ),
            _ => {}
        }
    })
    .connect(std::env::var("PLATFORM_NATS_URL").expect("development JetStream URL is configured"))
    .await
    .expect("dedicated NATS recovery identity connects");
    let recovery_jetstream = jetstream::new(recovery_nats_client);
    let failed_namespace = format!("e2e_failure_{}", Uuid::new_v4().simple());
    let failed_topology = EventTopology::from_namespace(Some(&failed_namespace))
        .expect("failure-path namespace is valid");
    let recovery_topology =
        EventTopology::from_namespace(Some("acceptance")).expect("recovery topology is valid");
    jetstream
        .get_stream(&recovery_topology.stream_name)
        .await
        .expect("pre-provisioned recovery stream exists in real JetStream");
    failed_topology
        .ensure_stream(&jetstream)
        .await
        .expect("temporary cleanup-test stream is created in real JetStream");
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
        runtime_pool,
        runtime_role,
        jetstream: &jetstream,
        event_topology: &event_topology,
    })
    .await;
    tenant_outbox_recovery::verify(tenant_outbox_recovery::TenantOutboxRecoveryContext {
        migration_pool,
        runtime_pool,
        recovery_jetstream: &recovery_jetstream,
        failed_topology: &failed_topology,
        recovery_topology: &recovery_topology,
        created_tenant_id,
        captured_events,
    })
    .await;
    identity_outbox_recovery::verify(
        migration_pool,
        runtime_pool,
        &recovery_jetstream,
        &failed_topology,
        &recovery_topology,
        captured_events,
    )
    .await;
    let event_delivery =
        tenant_event_delivery::verify(tenant_event_delivery::TenantEventDeliveryContext {
            client,
            migration_pool,
            runtime_pool,
            jetstream: &jetstream,
            publisher_jetstream: &e2e_jetstream,
            event_topology: &event_topology,
            event_namespace,
            http_addr,
            session_cookie,
            csrf_cookie,
            created_tenant_id,
            registered_user_id,
            legal_identification_number,
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
        e2e_jetstream,
        event_topology,
        stream,
        event_payload,
        registration_event_id,
        registration_subject,
        event_stream_cleanup,
        isolated_stream_cleanup,
    }
}
