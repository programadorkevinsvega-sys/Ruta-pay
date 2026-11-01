use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    matrix_operator_session_cookie: &str,
    matrix_operator_csrf_cookie: &str,
    runtime_pool: &PgPool,
    created_tenant_id: Uuid,
) {
    // A retained-journal gap must surface as a conflict at the HTTP edge,
    // not as a successful partial replay that silently skips an old event.
    let first_platform_event: Uuid =
        sqlx::query_scalar("SELECT event_id FROM realtime.event_journal WHERE platform_cursor=1")
            .fetch_one(migration_pool)
            .await
            .expect("first platform journal event exists for gap injection");
    sqlx::query("DELETE FROM realtime.event_journal WHERE event_id=$1")
        .bind(first_platform_event)
        .execute(migration_pool)
        .await
        .expect("first platform journal event is removed to simulate a retained gap");
    let platform_gap_replay = client
            .get(format!(
                "http://{http_addr}/v1/realtime/events?audience=platform&cursor=0"
            ))
            .header(
                header::COOKIE,
                format!(
                    "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
                ),
            )
            .send()
            .await
            .expect("platform replay reports a retained journal gap");
    assert_eq!(platform_gap_replay.status(), StatusCode::CONFLICT);
    let platform_gap_body: serde_json::Value = platform_gap_replay
        .json()
        .await
        .expect("realtime gap returns a safe error envelope");
    assert_eq!(platform_gap_body["error"]["code"], "realtime_cursor_gap");

    let matrix_operator_logout = client
            .delete(format!("http://{http_addr}/v1/platform/identity/session"))
            .header(header::ORIGIN, origin)
            .header(
                header::COOKIE,
                format!(
                    "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
                ),
            )
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .send()
            .await
            .expect("operator matrix session can be revoked");
    assert_eq!(matrix_operator_logout.status(), StatusCode::NO_CONTENT);
    let revoked_matrix_session_response = client
            .get(format!("http://{http_addr}/v1/platform/identity/session"))
            .header(
                header::COOKIE,
                format!(
                    "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
                ),
            )
            .send()
            .await
            .expect("revoked matrix operator session receives an HTTP response");
    assert_eq!(
        revoked_matrix_session_response.status(),
        StatusCode::UNAUTHORIZED
    );
    let revoked_operator_tenant_list = client
            .get(format!("http://{http_addr}/v1/platform/tenants?limit=10&offset=0"))
            .header(
                header::COOKIE,
                format!(
                    "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
                ),
            )
            .send()
            .await
            .expect("revoked operator session cannot access administrative tenant resources");
    assert_eq!(
        revoked_operator_tenant_list.status(),
        StatusCode::UNAUTHORIZED
    );
    super::outbox_leases::verify(migration_pool, runtime_pool, created_tenant_id).await;
}
