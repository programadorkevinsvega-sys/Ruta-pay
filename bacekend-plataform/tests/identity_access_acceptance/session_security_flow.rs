use platform_contracts::realtime::realtime_gateway_client::RealtimeGatewayClient;
use redis::aio::ConnectionManager;
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tonic::transport::Channel;

use super::operator_revocation_audit::MatrixOperatorCookies;

pub struct SessionSecurityFlowContext<'a> {
    pub client: &'a Client,
    pub realtime_client: &'a mut RealtimeGatewayClient<Channel>,
    pub migration_pool: &'a PgPool,
    pub redis_manager: &'a ConnectionManager,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub traceparent: &'a str,
    pub email: &'a str,
    pub ip_key: &'a str,
    pub captured_events: &'a Arc<Mutex<Vec<HashMap<String, String>>>>,
    pub default_headers: reqwest::header::HeaderMap,
    pub password: &'a str,
    pub operator_password: &'a str,
    pub operator_email: &'a str,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
    pub registered_user_id: uuid::Uuid,
    pub operator_id: uuid::Uuid,
    pub legal_identification_number: &'a str,
    pub second_legal_identification_number: &'a str,
    pub created_tenant_id: uuid::Uuid,
    pub replay_cursor: u64,
}

pub async fn verify(context: SessionSecurityFlowContext<'_>) -> MatrixOperatorCookies {
    let SessionSecurityFlowContext {
        client, realtime_client, migration_pool, redis_manager, http_addr, origin,
        traceparent, email, ip_key, captured_events, default_headers, password,
        operator_password, operator_email, session_cookie, csrf_cookie,
        operator_session_cookie, operator_csrf_cookie, registered_user_id,
        operator_id, legal_identification_number, second_legal_identification_number,
        created_tenant_id, replay_cursor,
    } = context;
    super::operator_realtime_revocation::verify(
        client,
        realtime_client,
        http_addr,
        origin,
        operator_session_cookie,
        operator_csrf_cookie,
    )
    .await;
    let matrix_operator_cookies = super::operator_revocation_audit::verify_and_reauthenticate(
        client,
        migration_pool,
        http_addr,
        origin,
        operator_id,
        operator_session_cookie,
        operator_csrf_cookie,
        operator_email,
        operator_password,
    )
    .await;
    let rejected_csrf = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", "wrong-csrf-token")
        .send()
        .await
        .expect("CSRF rejection response arrives");
    assert_eq!(rejected_csrf.status(), StatusCode::FORBIDDEN);
    super::tenant_realtime_revocation::verify(
        client,
        realtime_client,
        migration_pool,
        http_addr,
        origin,
        session_cookie,
        csrf_cookie,
        registered_user_id,
        created_tenant_id,
        replay_cursor,
    )
    .await;
    super::login_rate_limits::verify(
        client,
        redis_manager,
        http_addr,
        origin,
        traceparent,
        email,
        ip_key,
    )
    .await;
    super::rate_limit_telemetry::verify(captured_events);
    super::rate_limit_acceptance::verify(
        super::rate_limit_acceptance::RateLimitAcceptanceContext {
            http_addr,
            origin,
            default_headers,
            captured_events,
            password,
            operator_password,
            session_cookie,
            csrf_cookie,
            operator_session_cookie,
            operator_csrf_cookie,
            legal_identification_number,
            second_legal_identification_number,
        },
    )
    .await;
    matrix_operator_cookies
}
