use platform_contracts::identity_access::{
    AuthorizeApiKeyOperationRequest, identity_access_client::IdentityAccessClient,
};
use reqwest::{Client, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;
use std::net::SocketAddr;
use tonic::transport::Channel;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    authorization_client: &mut IdentityAccessClient<Channel>,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    created_tenant_id: Uuid,
    api_key_id: Uuid,
    api_key_secret: &str,
    api_key_created: &Value,
) {
    let foreign_api_key_list = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{}/api-keys",
            Uuid::new_v4()
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant key access to an unassociated tenant is rejected");
    assert_eq!(foreign_api_key_list.status(), StatusCode::FORBIDDEN);

    let api_key_list = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("API key metadata is available to the authorized tenant");
    assert_eq!(api_key_list.status(), StatusCode::OK);
    let api_key_metadata: serde_json::Value = api_key_list
        .json()
        .await
        .expect("API key metadata is safe JSON");
    assert_eq!(api_key_metadata[0]["status"], "active");
    assert_eq!(api_key_metadata[0]["prefix"], api_key_created["prefix"]);
    assert!(api_key_metadata[0].get("secret_once").is_none());
    assert!(api_key_metadata[0].get("secret_digest").is_none());

    let api_key_revoke = client
        .delete(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys/{api_key_id}"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", csrf_cookie)
        .header("x-correlation-id", "corr_api_key_revoke")
        .send()
        .await
        .expect("tenant owner can revoke its key through BFF and gRPC");
    assert_eq!(api_key_revoke.status(), StatusCode::NO_CONTENT);
    let csrf_rejected_api_key_revoke = client
        .delete(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys/{api_key_id}"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("missing CSRF token rejects API key revocation");
    assert_eq!(csrf_rejected_api_key_revoke.status(), StatusCode::FORBIDDEN);
    let revoked_key_status: String = sqlx::query_scalar(
        "SELECT status FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
    )
    .bind(created_tenant_id)
    .bind(api_key_id)
    .fetch_one(migration_pool)
    .await
    .expect("revocation persists in PostgreSQL");
    assert_eq!(revoked_key_status, "revoked");
    let revoked_api_key_authorization = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_revoked_denied".to_owned(),
        })
        .await
        .expect_err("revoked API key cannot authorize future operations");
    assert_eq!(
        revoked_api_key_authorization.code(),
        tonic::Code::Unauthenticated
    );
    let api_key_audit: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE resource_id=$1 AND action='api_key.created' AND outcome='completed') AND EXISTS (SELECT 1 FROM identity_access.audit_events WHERE resource_id=$1 AND action='api_key.revoked' AND outcome='completed')",
        )
        .bind(api_key_id)
        .fetch_one(migration_pool)
        .await
        .expect("API key creation and revocation are audited");
    assert!(api_key_audit);
}
