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

pub async fn verify(
    client: &Client,
    realtime_client: &mut RealtimeGatewayClient<Channel>,
    migration_pool: &PgPool,
    redis_manager: &ConnectionManager,
    http_addr: SocketAddr,
    origin: &str,
    traceparent: &str,
    email: &str,
    ip_key: &str,
    captured_events: &Arc<Mutex<Vec<HashMap<String, String>>>>,
    default_headers: reqwest::header::HeaderMap,
    password: &str,
    operator_password: &str,
    operator_email: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
    registered_user_id: uuid::Uuid,
    operator_id: uuid::Uuid,
    legal_identification_number: &str,
    second_legal_identification_number: &str,
) -> MatrixOperatorCookies {
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
