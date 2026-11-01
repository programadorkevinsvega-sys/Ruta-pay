use super::{
    api_key_acceptance_flow, cleanup_fixture, environment_fixture, event_fixture,
    is_stream_not_found, platform_replay_and_logout, realtime_acceptance_flow,
    second_tenant_flow, session_security_flow, tenant_admin_acceptance_flow,
    tenant_onboarding_phase::{self, TenantOnboardingFixture}, IdentityEnvironment,
};

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
        database_cleanup,
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
        tenant_slug,
        tenant_admin_status_body,
        ..
    } = tenant;
    let http_addr = servers.http_addr;
    let origin = servers.origin.clone();
    let default_headers = servers.default_headers.clone();
    let client = servers.client.clone();
    let mut authorization_client = servers.authorization_client.clone();
    let mut realtime_acceptance_client = servers.realtime_acceptance_client.clone();

    let event_fixture = event_fixture::create(event_fixture::EventFixtureContext {
        migration_pool: &migration_pool,
        runtime_pool: &runtime_pool,
        runtime_role: &runtime_role,
        client: &client,
        http_addr,
        session_cookie: &session_cookie,
        csrf_cookie: &csrf_cookie,
        created_tenant_id,
        registered_user_id,
        legal_identification_number: &legal_identification_number,
    })
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
        api_key_acceptance_flow::ApiKeyAcceptanceContext {
            client: &client,
            migration_pool: &migration_pool,
            authorization_client: &mut authorization_client,
            http_addr,
            grpc_addr: servers.grpc_addr,
            origin: &origin,
            session_cookie: &session_cookie,
            csrf_cookie: &csrf_cookie,
            operator_session_cookie: &operator_session_cookie,
            operator_csrf_cookie: &operator_csrf_cookie,
            created_tenant_id,
            registered_user_id,
        },
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
        &operator_session_cookie,
        &operator_csrf_cookie,
    )
    .await;

    let matrix_operator_cookies = session_security_flow::verify(
        session_security_flow::SessionSecurityFlowContext {
            client: &client,
            realtime_client: &mut realtime_acceptance_client,
            migration_pool: &migration_pool,
            redis_manager: &redis_manager,
            http_addr,
            origin: &origin,
            traceparent: &traceparent,
            email: &email,
            ip_key: &ip_key,
            captured_events: &captured_events,
            default_headers,
            password: &password,
            operator_password: &operator_password,
            operator_email: &operator_email,
            session_cookie: &session_cookie,
            csrf_cookie: &csrf_cookie,
            operator_session_cookie: &operator_session_cookie,
            operator_csrf_cookie: &operator_csrf_cookie,
            registered_user_id,
            operator_id,
            legal_identification_number: &legal_identification_number,
            second_legal_identification_number: &second_legal_identification_number,
            created_tenant_id,
            replay_cursor,
        },
    )
    .await;
    let matrix_operator_session_cookie = matrix_operator_cookies.session_cookie;
    let matrix_operator_csrf_cookie = matrix_operator_cookies.csrf_cookie;

    tenant_admin_acceptance_flow::verify(
        tenant_admin_acceptance_flow::TenantAdminAcceptanceContext {
            client: &client,
            migration_pool: &migration_pool,
            authorization_client: &mut authorization_client,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            operator_session_cookie: &operator_session_cookie,
            operator_csrf_cookie: &operator_csrf_cookie,
            operator_id,
            created_tenant_id,
            session_cookie: &session_cookie,
            csrf_cookie: &csrf_cookie,
            second_session_cookie: &second_session_cookie,
            second_csrf_cookie: &second_csrf_cookie,
            second_tenant_id: second_tenant.tenant_id,
            tenant_status_version: tenant_admin_status_body["status_version"].clone(),
        },
    )
    .await;

    platform_replay_and_logout::verify(
        platform_replay_and_logout::PlatformReplayAndLogoutContext {
            client: &client,
            migration_pool: &migration_pool,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            runtime_pool: &runtime_pool,
            created_tenant_id,
        },
    )
    .await;

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
