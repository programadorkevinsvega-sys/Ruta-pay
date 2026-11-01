use reqwest::{Client, StatusCode, header};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct SessionSecurityContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
    pub second_session_cookie: &'a str,
    pub second_csrf_cookie: &'a str,
    pub second_tenant_id: Uuid,
    pub created_tenant_id: Uuid,
    pub tenant_status_version: serde_json::Value,
}

pub async fn verify(context: SessionSecurityContext<'_>) {
    let SessionSecurityContext {
        client,
        migration_pool,
        http_addr,
        origin,
        session_cookie,
        csrf_cookie,
        operator_session_cookie,
        operator_csrf_cookie,
        second_session_cookie,
        second_csrf_cookie,
        second_tenant_id,
        created_tenant_id,
        tenant_status_version,
    } = context;
    // Mutating HTTP endpoints must reject missing CSRF tokens or untrusted origins
    // before reaching a module or changing durable state.
    let tenants_before_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(migration_pool)
            .await
            .expect("tenant count before CSRF rejection is queryable");
    let csrf_rejected_tenant_create = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .json(&serde_json::json!({
            "display_name": "Must not be created",
            "slug": format!("csrf-rejected-{}", Uuid::new_v4().simple()),
            "legal_identification_type": "nit",
            "legal_identification_number": format!("8{:018}", Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000)
        }))
        .send()
        .await
        .expect("missing CSRF token receives an HTTP response");
    assert_eq!(csrf_rejected_tenant_create.status(), StatusCode::FORBIDDEN);
    let tenants_after_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(migration_pool)
            .await
            .expect("tenant count after CSRF rejection is queryable");
    assert_eq!(tenants_after_csrf_rejection, tenants_before_csrf_rejection);

    let keys_before_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(migration_pool)
            .await
            .expect("API key count before CSRF rejection is queryable");
    let csrf_rejected_api_key = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .json(&serde_json::json!({
            "name": "Must not be created",
            "environment": "test",
            "scopes": ["payment_intents.create"]
        }))
        .send()
        .await
        .expect("missing CSRF token rejects API key creation");
    assert_eq!(csrf_rejected_api_key.status(), StatusCode::FORBIDDEN);
    let keys_after_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(migration_pool)
            .await
            .expect("API key count after CSRF rejection is queryable");
    assert_eq!(keys_after_csrf_rejection, keys_before_csrf_rejection);

    let csrf_rejected_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, "https://attacker.example")
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Must not be applied",
            "expected_version": tenant_status_version
        }))
        .send()
        .await
        .expect("untrusted Origin rejects status mutation");
    assert_eq!(csrf_rejected_status_change.status(), StatusCode::FORBIDDEN);

    let csrf_rejected_tenant_logout = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("missing CSRF token rejects tenant logout");
    assert_eq!(csrf_rejected_tenant_logout.status(), StatusCode::FORBIDDEN);

    let csrf_rejected_operator_logout = client
        .delete(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .send()
        .await
        .expect("missing CSRF token rejects operator logout");
    assert_eq!(
        csrf_rejected_operator_logout.status(),
        StatusCode::FORBIDDEN
    );

    // Expired and revoked sessions are rejected at the HTTP boundary.
    // The second tenant session has not been revoked; expire it explicitly so
    // this assertion distinguishes expiration from the separate logout checks.
    let expired_session_digest = Sha256::digest(second_session_cookie.as_bytes());
    sqlx::query(
        "UPDATE identity_access.sessions SET expires_at=now()-interval '1 second' WHERE token_digest=$1",
    )
    .bind(expired_session_digest.as_slice())
    .execute(migration_pool)
    .await
    .expect("active second tenant session is marked expired in isolated PostgreSQL");
    let expired_session_response = client
        .get(format!("http://{http_addr}/v1/identity/session"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .send()
        .await
        .expect("expired session receives an HTTP response");
    assert_eq!(expired_session_response.status(), StatusCode::UNAUTHORIZED);
    let expired_session_api_key_access = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{second_tenant_id}/api-keys"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .send()
        .await
        .expect("expired tenant session cannot access API key resources");
    assert_eq!(
        expired_session_api_key_access.status(),
        StatusCode::UNAUTHORIZED
    );
}
