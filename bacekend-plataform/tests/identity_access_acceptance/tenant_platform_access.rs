use reqwest::{Client, StatusCode, header};
use std::net::SocketAddr;
use uuid::Uuid;

pub struct TenantPlatformAccessContext<'a> {
    pub client: &'a Client,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
}

pub async fn verify(context: TenantPlatformAccessContext<'_>) {
    let TenantPlatformAccessContext {
        client, http_addr, origin, session_cookie, csrf_cookie,
        operator_session_cookie, operator_csrf_cookie, created_tenant_id,
    } = context;
    let tenant_session_platform_list = client
        .get(format!("http://{http_addr}/v1/platform/tenants?limit=10&offset=0"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant session cannot enumerate platform tenants");
    assert_eq!(tenant_session_platform_list.status(), StatusCode::UNAUTHORIZED);
    let tenant_session_platform_mutation = client
        .post(format!("http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Tenant cannot administer platform state",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("tenant session cannot mutate platform tenant state");
    assert_eq!(tenant_session_platform_mutation.status(), StatusCode::FORBIDDEN);
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
