use super::*;

pub struct TenantOnboardingFixture {
    pub session_cookie: String,
    pub csrf_cookie: String,
    pub traceparent: String,
    pub ip_key: String,
    pub operator_session_cookie: String,
    pub operator_csrf_cookie: String,
    pub legal_identification_number: String,
    pub created_tenant_id: Uuid,
    pub tenant_body: serde_json::Value,
    pub tenant_slug: String,
    pub tenant_admin_status_body: serde_json::Value,
}

pub async fn setup(environment: &IdentityEnvironment) -> TenantOnboardingFixture {
    let client = &environment.servers.client;
    let migration_pool = &environment.migration_pool;
    let http_addr = environment.servers.http_addr;
    let origin = &environment.servers.origin;
    let operator_email = &environment.operator_email;
    let operator_password = &environment.operator_password;
    let operator_id = environment.operator_id;
    let mut authorization_client = environment.servers.authorization_client.clone();
    let email = &environment.email;
    let password = &environment.password;
    let test_source_ip = environment.servers.test_source_ip;
    let redis_manager = &environment.redis_manager;
    let captured_events = &environment.captured_events;
    let registered_user_id = environment.registered_user_id;
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
    TenantOnboardingFixture {
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
    }
}
