use super::*;
use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

use super::identity_access_acceptance_support::EventStreamCleanup;

pub(super) struct TenantEventDeliveryContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub runtime_pool: &'a PgPool,
    pub jetstream: &'a jetstream::Context,
    pub event_topology: &'a EventTopology,
    pub event_namespace: String,
    pub http_addr: SocketAddr,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
    pub registered_user_id: Uuid,
    pub legal_identification_number: &'a str,
}

pub(super) struct TenantEventDeliveryResult {
    pub(super) stream: jetstream::stream::Stream,
    pub(super) event_payload: Vec<u8>,
    pub(super) registration_event_id: Uuid,
    pub(super) registration_subject: String,
    pub(super) event_stream_cleanup: EventStreamCleanup,
    pub(super) isolated_stream_cleanup: EventStreamCleanup,
}

pub(super) async fn verify(context: TenantEventDeliveryContext<'_>) -> TenantEventDeliveryResult {
    let TenantEventDeliveryContext {
        client,
        migration_pool,
        runtime_pool,
        jetstream,
        event_topology,
        event_namespace,
        http_addr,
        session_cookie,
        csrf_cookie,
        created_tenant_id,
        registered_user_id,
        legal_identification_number,
    } = context;
    let stream = event_topology
        .ensure_stream(jetstream)
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
        .ensure_stream(jetstream)
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
    .fetch_one(migration_pool)
    .await
    .expect("onboarding event identifier is durable");
    let published_count = platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
        runtime_pool,
        jetstream,
        event_topology.subject_prefix,
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
    .fetch_one(migration_pool)
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
    .fetch_one(migration_pool)
    .await
    .expect("outbox publication state can be verified");
    assert!(
        outbox_published,
        "Outbox is marked only after JetStream ack"
    );
    let event_message = stream
        .get_last_raw_message_by_subject(&registration_subject)
        .await
        .expect("JetStream durably stores tenant registration event");
    let event_envelope: serde_json::Value = serde_json::from_slice(&event_message.payload)
        .expect("published event uses the versioned JSON envelope");
    assert_eq!(
        event_envelope["event_id"],
        registration_event_id.to_string()
    );
    assert!(
        !String::from_utf8_lossy(&event_message.payload).contains(&legal_identification_number)
    );
    let membership_created = platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
        runtime_pool,
        &event_message.payload,
    )
    .await
    .expect("real consumer provisions owner membership from the durable event");
    assert!(membership_created);
    let owner_membership: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.tenant_memberships WHERE tenant_id=$1 AND user_id=$2 AND role='tenant_owner' AND status='active')",
    )
    .bind(created_tenant_id)
    .bind(registered_user_id)
    .fetch_one(migration_pool)
    .await
    .expect("owner membership is persisted in its owning module");
    assert!(owner_membership);
    let duplicate_membership = platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
        runtime_pool,
        &event_message.payload,
    )
    .await
    .expect("redelivery is handled idempotently");
    assert!(
        !duplicate_membership,
        "redelivered event does not duplicate membership"
    );
    let memberships_response = client
        .get(format!("http://{http_addr}/v1/identity/memberships"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_memberships_read")
        .send()
        .await
        .expect("tenant membership query reaches identity module through BFF");
    assert_eq!(memberships_response.status(), StatusCode::OK);
    let memberships_body: serde_json::Value = memberships_response
        .json()
        .await
        .expect("membership response is safe JSON");
    assert_eq!(
        memberships_body["memberships"][0]["tenant_id"],
        created_tenant_id.to_string()
    );
    assert_eq!(memberships_body["memberships"][0]["role"], "tenant_owner");
    TenantEventDeliveryResult {
        stream,
        event_payload: event_message.payload.to_vec(),
        registration_event_id,
        registration_subject,
        event_stream_cleanup,
        isolated_stream_cleanup,
    }
}
