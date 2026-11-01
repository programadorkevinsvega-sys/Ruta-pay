mod api_key_creation;
mod api_key_dto_validation;
mod concurrent_registration;
mod credential_validation;
mod database_fixture;
mod database_migrations;
mod duplicate_registration;
mod event_fixture;
mod identity_outbox_recovery;
mod operator_redis_fixture;
mod operator_revocation_audit;
mod operator_session_acceptance;
mod operator_role_matrix;
mod outbox_database_failures;
mod outbox_leases;
mod outbox_transaction_rollback;
#[path = "identity_access_acceptance/phase3_catalog.rs"]
mod phase3_catalog;
#[path = "identity_access_acceptance/phase3_entitlements.rs"]
mod phase3_entitlements;
mod platform_realtime_replay;
mod operator_realtime_revocation;
mod tenant_realtime_replay;
mod tenant_realtime_revocation;
mod login_rate_limits;
mod platform_replay_and_logout;
mod rate_limit_acceptance;
mod rate_limit_telemetry;
mod realtime_acceptance_flow;
mod realtime_journal_acceptance;
mod api_key_acceptance_flow;
mod api_key_authorization;
mod api_key_lifecycle;
mod second_tenant_flow;
mod second_tenant_isolation;
mod second_tenant_registration;
mod secondary_tenant_lifecycle;
mod server_fixture;
mod session_security;
mod session_security_flow;
mod tenant_dto_validation;
mod tenant_event_delivery;
mod tenant_outbox_recovery;
mod tenant_platform_access;
mod tenant_registration_acceptance;
mod tenant_registration_races;
mod tenant_session_acceptance;
mod tenant_role_matrix;
mod tenant_status_input_validation;
mod tenant_status_lifecycle;
mod tenant_status_outbox_acceptance;
mod tenant_status_race;
mod tenant_status_version_validation;

use async_nats::jetstream;
#[path = "identity_access_acceptance/support.rs"]
mod identity_access_acceptance_support;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use identity_access_acceptance_support::{
    AcceptanceDatabaseCleanup, EventCapture, EventStreamCleanup, cookie_value, is_stream_not_found,
};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient as CatalogPricingGrpcClient,
    entitlements_quotas::entitlements_quotas_client::EntitlementsQuotasClient as EntitlementsQuotasGrpcClient,
    identity_access::{
        AuthorizeApiKeyOperationRequest, AuthorizeOperatorOperationRequest,
        AuthorizeTenantOperationRequest, identity_access_client::IdentityAccessClient,
    },
    realtime::{
        SubscribeEventsRequest,
        realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient,
    },
    runtime::runtime_probe_client::RuntimeProbeClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient,
};
use platform_identity_access::application::operator_bootstrap::OperatorBootstrapService;
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use reqwest::{Client, StatusCode, header};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, sync::oneshot};
use tracing_subscriber::prelude::*;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn identity_http_grpc_postgres_redis_cookie_csrf_rate_limit_and_cleanup() {
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    let captured = EventCapture::default();
    let captured_events = captured.0.clone();
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("identity-acceptance")))
        .with(captured)
        .try_init()
        .expect("identity acceptance subscriber initializes");

    let database = database_fixture::create().await;
    let admin_pool = database.admin_pool;
    let migration_pool = database.migration_pool;
    let runtime_pool = database.runtime_pool;
    let runtime_role = database.runtime_role;
    let redis_url = database.redis_url;
    let mut database_cleanup = database.cleanup;

    let operator_fixture = operator_redis_fixture::bootstrap_and_verify_redis_failure(
        &runtime_pool,
        &migration_pool,
        &redis_url,
    )
    .await;
    let operator_email = operator_fixture.operator_email;
    let operator_password = operator_fixture.operator_password;
    let operator_id = operator_fixture.operator_id;
    let redis_client = operator_fixture.redis_client;
    let redis_manager = operator_fixture.redis_manager;

    let servers = server_fixture::ServerFixture::start(
        &runtime_pool,
        &redis_manager,
        redis_client,
    )
    .await;
    let grpc_addr = servers.grpc_addr;
    let http_addr = servers.http_addr;
    let origin = servers.origin.clone();
    let default_headers = servers.default_headers.clone();
    let test_source_ip = servers.test_source_ip;
    let client = servers.client.clone();
    let mut authorization_client = servers.authorization_client.clone();
    let mut realtime_acceptance_client = servers.realtime_acceptance_client.clone();

    let email = format!("acceptance_{}@example.test", Uuid::new_v4().simple());
    let password = format!("A-valid-password-{}!", Uuid::new_v4().simple());
    credential_validation::verify(credential_validation::IdentityCredentialValidationContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        email: &email,
        password: &password,
    })
    .await;

    let register_response = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_register")
        .json(&serde_json::json!({"email": email, "password": password}))
        .send()
        .await
        .expect("HTTP registration response arrives");
    assert_eq!(register_response.status(), StatusCode::CREATED);

    let registration: serde_json::Value = register_response
        .json()
        .await
        .expect("registration response contains its resource identifier");
    let registered_user_id = Uuid::parse_str(
        registration["user_id"]
            .as_str()
            .expect("registration user id is present"),
    )
    .expect("registration user id is valid");

    outbox_transaction_rollback::verify(&client, &migration_pool, http_addr, &origin).await;
    duplicate_registration::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &email,
        &password,
        registered_user_id,
    )
    .await;
    concurrent_registration::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        registered_user_id,
    )
    .await;

    let tenant_cookies = tenant_session_acceptance::verify(
        &client,
        http_addr,
        &origin,
        &email,
        &password,
        test_source_ip,
        &redis_manager,
        &captured_events,
        registered_user_id,
    )
    .await;
    let session_cookie = tenant_cookies.session_cookie.clone();
    let csrf_cookie = tenant_cookies.csrf_cookie.clone();
    let traceparent = tenant_cookies.traceparent.clone();
    let ip_key = tenant_cookies.ip_key.clone();
    let operator_cookies = operator_session_acceptance::verify(
        &client,
        http_addr,
        &origin,
        &operator_email,
        &operator_password,
        operator_id,
        &mut authorization_client,
        &migration_pool,
        &tenant_cookies,
    )
    .await;
    let operator_session_cookie = operator_cookies.session_cookie;
    let operator_csrf_cookie = operator_cookies.csrf_cookie;

    let tenant_registration = tenant_registration_acceptance::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
    )
    .await;
    let legal_identification_number = tenant_registration.legal_identification_number;
    let created_tenant_id = tenant_registration.tenant_id;
    let tenant_body = tenant_registration.tenant_body;
    let tenant_slug = tenant_registration.tenant_slug;

    tenant_platform_access::verify(
        &client,
        http_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
        &operator_session_cookie,
        &operator_csrf_cookie,
        created_tenant_id,
    )
    .await;
    let tenant_admin_status_body = tenant_status_outbox_acceptance::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &operator_session_cookie,
        &operator_csrf_cookie,
        created_tenant_id,
        &tenant_body,
    )
    .await;

    let event_fixture = event_fixture::create(
        &migration_pool,
        &runtime_pool,
        &runtime_role,
        &client,
        http_addr,
        &session_cookie,
        &csrf_cookie,
        created_tenant_id,
        registered_user_id,
        &legal_identification_number,
    )
    .await;
    let jetstream = event_fixture.jetstream;
    let _failed_topology = event_fixture.failed_topology;
    let event_topology = event_fixture.event_topology;
    let stream = event_fixture.stream;
    let event_payload = event_fixture.event_payload;
    let registration_event_id = event_fixture.registration_event_id;
    let registration_subject = event_fixture.registration_subject;
    let mut event_stream_cleanup = event_fixture.event_stream_cleanup;
    let mut isolated_stream_cleanup = event_fixture.isolated_stream_cleanup;

    api_key_acceptance_flow::verify(
        &client,
        &migration_pool,
        &mut authorization_client,
        http_addr,
        grpc_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
        &operator_session_cookie,
        &operator_csrf_cookie,
        created_tenant_id,
        registered_user_id,
    )
    .await;

    let second_tenant = second_tenant_flow::verify(
        &client,
        &migration_pool,
        &runtime_pool,
        &jetstream,
        &event_topology,
        &stream,
        &registration_subject,
        http_addr,
        &origin,
        &tenant_slug,
        created_tenant_id,
    )
    .await;
    let second_session_cookie = second_tenant.credentials.session_cookie;
    let second_csrf_cookie = second_tenant.credentials.csrf_cookie;
    let second_legal_identification_number =
        second_tenant.credentials.legal_identification_number;
    let second_tenant_id = second_tenant.tenant_id;

    let replay_cursor = realtime_acceptance_flow::verify(
        &redis_manager,
        &runtime_pool,
        &migration_pool,
        &event_payload,
        registration_event_id,
        &legal_identification_number,
        &client,
        http_addr,
        &session_cookie,
        &csrf_cookie,
        created_tenant_id,
        &stream,
        &event_topology,
        &mut realtime_acceptance_client,
        &operator_session_cookie,
        &operator_csrf_cookie,
    )
    .await;

    let matrix_operator_cookies = session_security_flow::verify(
        &client,
        &mut realtime_acceptance_client,
        &migration_pool,
        &redis_manager,
        http_addr,
        &origin,
        &traceparent,
        &email,
        &ip_key,
        &captured_events,
        default_headers,
        &password,
        &operator_password,
        &operator_email,
        &session_cookie,
        &csrf_cookie,
        &operator_session_cookie,
        &operator_csrf_cookie,
        registered_user_id,
        operator_id,
        &legal_identification_number,
        &second_legal_identification_number,
    )
    .await;
    let matrix_operator_session_cookie = matrix_operator_cookies.session_cookie;
    let matrix_operator_csrf_cookie = matrix_operator_cookies.csrf_cookie;

    operator_role_matrix::verify(operator_role_matrix::OperatorRoleMatrixContext {
        client: &client,
        migration_pool: &migration_pool,
        authorization_client: &mut authorization_client,
        operator_id,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        created_tenant_id,
        http_addr,
    })
    .await;

    tenant_status_lifecycle::verify(tenant_status_lifecycle::TenantStatusLifecycleContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
    })
    .await;

    let (tenant_before_invalid_transitions, current_status_version) =
        tenant_status_input_validation::verify(
            tenant_status_input_validation::TenantStatusInputContext {
                client: &client,
                migration_pool: &migration_pool,
                http_addr,
                origin: &origin,
                matrix_operator_session_cookie: &matrix_operator_session_cookie,
                matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
                created_tenant_id,
            },
        )
        .await;
    tenant_status_version_validation::verify(
        tenant_status_version_validation::TenantStatusVersionContext {
            client: &client,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            created_tenant_id,
            current_status_version,
        },
    )
    .await;
    tenant_dto_validation::verify(tenant_dto_validation::TenantDtoValidationContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        second_session_cookie: &second_session_cookie,
        second_csrf_cookie: &second_csrf_cookie,
        created_tenant_id,
        tenant_before_invalid_transitions,
    })
    .await;

    secondary_tenant_lifecycle::verify(
        secondary_tenant_lifecycle::SecondaryTenantLifecycleContext {
            client: &client,
            migration_pool: &migration_pool,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            operator_id,
            second_tenant_id,
        },
    )
    .await;

    session_security::verify(session_security::SessionSecurityContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        session_cookie: &session_cookie,
        csrf_cookie: &csrf_cookie,
        operator_session_cookie: &operator_session_cookie,
        operator_csrf_cookie: &operator_csrf_cookie,
        second_session_cookie: &second_session_cookie,
        second_csrf_cookie: &second_csrf_cookie,
        second_tenant_id,
        created_tenant_id,
        tenant_status_version: tenant_admin_status_body["status_version"].clone(),
    })
    .await;

    tenant_status_race::verify(tenant_status_race::TenantStatusRaceContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
        current_status_version,
    })
    .await;

    platform_replay_and_logout::verify(
        &client,
        &migration_pool,
        http_addr,
        &matrix_operator_session_cookie,
        &matrix_operator_csrf_cookie,
    )
    .await;

    outbox_leases::verify(&migration_pool, &runtime_pool, created_tenant_id).await;

    let mut isolated_stream_cleanup = isolated_stream_cleanup;
    isolated_stream_cleanup.cleanup().await;
    let mut event_stream_cleanup = event_stream_cleanup;
    event_stream_cleanup.cleanup().await;
    assert!(
        jetstream
            .get_stream(&event_topology.stream_name)
            .await
            .is_err_and(is_stream_not_found)
    );

    servers.shutdown().await;
    runtime_pool.close().await;
    migration_pool.close().await;
    database_cleanup.cleanup().await;
    admin_pool.close().await;
    provider.shutdown().expect("trace provider shuts down");
}
