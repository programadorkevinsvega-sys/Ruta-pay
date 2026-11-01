use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    http_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
    created_tenant_id: Uuid,
) {
        let tenant_session_platform_list = client
            .get(format!(
                "http://{http_addr}/v1/platform/tenants?limit=10&offset=0"
            ))
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .send()
            .await
            .expect("tenant session cannot enumerate platform tenants");
        assert_eq!(
            tenant_session_platform_list.status(),
            StatusCode::UNAUTHORIZED
        );
        let tenant_session_platform_mutation = client
            .post(format!(
                "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .json(&serde_json::json!({
                "status": "active",
                "reason": "Tenant cannot administer platform state",
                "expected_version": 1
            }))
            .send()
            .await
            .expect("tenant session cannot mutate platform tenant state");
        assert_eq!(
            tenant_session_platform_mutation.status(),
            StatusCode::FORBIDDEN
        );
        let tenant_list = client
            .get(format!("http://{http_addr}/v1/platform/tenants?query=Acceptance&limit=10&offset=0"))
            .header(
                header::COOKIE,
                format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
            )
            .header("x-correlation-id", "corr_tenant_admin_list")
            .send()
            .await
            .expect("operator can list tenants through BFF and gRPC");
        assert_eq!(tenant_list.status(), StatusCode::OK);
        let tenant_list_body: serde_json::Value =
            tenant_list.json().await.expect("tenant list is JSON");
        assert_eq!(tenant_list_body["total"], 1);
}
