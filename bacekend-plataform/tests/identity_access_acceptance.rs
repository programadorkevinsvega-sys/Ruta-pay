#[path = "identity_access_acceptance/modules.rs"]
mod acceptance_modules;
use acceptance_modules::*;

#[tokio::test(flavor = "multi_thread")]
async fn identity_http_grpc_postgres_redis_cookie_csrf_rate_limit_and_cleanup() {
    let environment = environment_fixture::setup().await;
    let tenant = tenant_onboarding_phase::setup(&environment).await;
    let IdentityEnvironment {
        provider,
        captured_events,
        admin_pool,
        migration_pool,
        runtime_pool,
        runtime_role,
        mut database_cleanup,
        redis_manager,
        operator_email,
        operator_password,
        operator_id,
        servers,
        email,
        password,
        registered_user_id,
    } = environment;
    let TenantOnboardingFixture {
        session_cookie,
        csrf_cookie,
        traceparent,
        ip_key,
        operator_session_cookie,
        operator_csrf_cookie,
        legal_identification_number,
        created_tenant_id,
        tenant_body,
        tenant_slug,
        tenant_admin_status_body,
    } = tenant;
    let grpc_addr = servers.grpc_addr;
    let http_addr = servers.http_addr;
    let origin = servers.origin.clone();
    let default_headers = servers.default_headers.clone();
    let client = servers.client.clone();
    let mut authorization_client = servers.authorization_client.clone();
    let mut realtime_acceptance_client = servers.realtime_acceptance_client.clone();

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
    let second_legal_identification_number = second_tenant.credentials.legal_identification_number;
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

    tenant_admin_acceptance_flow::verify(
        &client,
        &migration_pool,
        &mut authorization_client,
        http_addr,
        &origin,
        &matrix_operator_session_cookie,
        &matrix_operator_csrf_cookie,
        &operator_session_cookie,
        &operator_csrf_cookie,
        operator_id,
        created_tenant_id,
        registered_user_id,
        &second_session_cookie,
        &second_csrf_cookie,
        second_tenant_id,
        tenant_admin_status_body["status_version"].clone(),
    )
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

    isolated_stream_cleanup.cleanup().await;
    event_stream_cleanup.cleanup().await;
    assert!(
        jetstream
            .get_stream(&event_topology.stream_name)
            .await
            .is_err_and(is_stream_not_found)
    );

    cleanup_fixture::shutdown(
        servers,
        runtime_pool,
        migration_pool,
        database_cleanup,
        admin_pool,
        provider,
    )
    .await;
}
