use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

use super::identity_access_acceptance_support::cookie_value;

pub struct MatrixOperatorCookies {
    pub session_cookie: String,
    pub csrf_cookie: String,
}

pub async fn verify_and_reauthenticate(
    client: &Client,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    operator_id: Uuid,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
    operator_email: &str,
    operator_password: &str,
) -> MatrixOperatorCookies {
    let operator_logout_audited: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'identity.session.revoked' AND correlation_id = 'corr_operator_logout')",
        )
        .bind(operator_id)
        .fetch_one(&migration_pool)
        .await
        .expect("operator session revocation audit is queryable");
    assert!(
        operator_logout_audited,
        "operator session revocation is auditable"
    );
    let session_revocation_event: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM identity_access.outbox_events WHERE event_type = 'identity.session.revoked' AND audience = 'platform_operator')",
        )
        .fetch_one(&migration_pool)
        .await
        .expect("operator session revocation event is queryable");
    assert!(
        session_revocation_event,
        "operator session revocation reaches durable outbox"
    );
    let operator_replay = client
            .get(format!("http://{http_addr}/v1/platform/identity/session"))
            .header(
                header::COOKIE,
                format!(
                    "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
                ),
            )
            .send()
            .await
            .expect("revoked operator session replay is rejected");
    assert_eq!(operator_replay.status(), StatusCode::UNAUTHORIZED);

    // The preceding logout deliberately revoked the first operator session.
    // Create a fresh session for the role/permission matrix that follows.
    let matrix_operator_login = client
        .post(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_operator_matrix_login")
        .json(&serde_json::json!({
            "email": operator_email.clone(),
            "password": operator_password.clone()
        }))
        .send()
        .await
        .expect("operator matrix login response arrives");
    assert_eq!(matrix_operator_login.status(), StatusCode::OK);
    let matrix_operator_cookies: Vec<String> = matrix_operator_login
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let matrix_operator_session_cookie =
        cookie_value(&matrix_operator_cookies, "payvia_platform_session");
    let matrix_operator_csrf_cookie =
        cookie_value(&matrix_operator_cookies, "payvia_platform_csrf");
    MatrixOperatorCookies {
        session_cookie: matrix_operator_session_cookie,
        csrf_cookie: matrix_operator_csrf_cookie,
    }
}
