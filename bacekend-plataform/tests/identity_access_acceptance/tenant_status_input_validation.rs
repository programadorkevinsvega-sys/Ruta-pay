use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct TenantStatusInputContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
}

pub async fn verify(context: TenantStatusInputContext<'_>) -> ((String, i64), i64) {
    let TenantStatusInputContext {
        client,
        migration_pool,
        http_addr,
        origin,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        created_tenant_id,
    } = context;
    // Invalid reasons and stale optimistic-lock versions must be rejected
    // before changing the tenant aggregate or its version. Read the committed
    // version instead of duplicating assumptions about prior transitions.
    let tenant_before_invalid_transitions: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(migration_pool)
            .await
            .expect("tenant state before invalid transitions is queryable");
    assert_eq!(tenant_before_invalid_transitions.0, "active");
    let current_status_version = tenant_before_invalid_transitions.1;
    assert!(current_status_version > 0);

    let missing_status_reason = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", matrix_operator_csrf_cookie)
        .header("x-correlation-id", "corr_tenant_status_missing_reason")
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": " ",
            "expected_version": current_status_version
        }))
        .send()
        .await
        .expect("status transition without a meaningful reason is rejected");
    assert_eq!(missing_status_reason.status(), StatusCode::BAD_REQUEST);
    let invalid_reasons = vec!["ab".to_owned(), "r".repeat(501)];
    for invalid_reason in invalid_reasons {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
            ))
            .header(header::ORIGIN, origin)
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .header("x-csrf-token", matrix_operator_csrf_cookie)
            .json(&serde_json::json!({
                "status": "suspended",
                "reason": invalid_reason,
                "expected_version": current_status_version
            }))
            .send()
            .await
            .expect("out-of-range lifecycle reason receives an HTTP response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    (tenant_before_invalid_transitions, current_status_version)
}
