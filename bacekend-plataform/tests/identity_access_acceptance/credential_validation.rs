use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;

pub struct IdentityCredentialValidationContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub email: &'a str,
    pub password: &'a str,
}

pub async fn verify(context: IdentityCredentialValidationContext<'_>) {
    let IdentityCredentialValidationContext {
        client,
        migration_pool,
        http_addr,
        origin,
        email,
        password,
    } = context;
    let identity_state_before_origin_rejection: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM identity_access.tenant_users),(SELECT count(*) FROM identity_access.sessions)",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity and session counts before Origin rejection are queryable");
    let invalid_origin_registration = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, "https://attacker.example")
        .json(&serde_json::json!({
            "email": email.clone(),
            "password": password.clone()
        }))
        .send()
        .await
        .expect("untrusted Origin registration receives an HTTP response");
    assert_eq!(invalid_origin_registration.status(), StatusCode::FORBIDDEN);
    let invalid_origin_tenant_login = client
        .post(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, "https://attacker.example")
        .json(&serde_json::json!({
            "email": email.clone(),
            "password": password.clone()
        }))
        .send()
        .await
        .expect("untrusted Origin tenant login receives an HTTP response");
    assert_eq!(invalid_origin_tenant_login.status(), StatusCode::FORBIDDEN);
    let identity_state_after_origin_rejection: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM identity_access.tenant_users),(SELECT count(*) FROM identity_access.sessions)",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity and session counts after Origin rejection are queryable");
    assert_eq!(
        identity_state_after_origin_rejection, identity_state_before_origin_rejection,
        "untrusted Origin must not create identities or sessions"
    );
    let identities_before_invalid_credentials: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.tenant_users")
            .fetch_one(&migration_pool)
            .await
            .expect("identity count before invalid credentials is queryable");
    let invalid_credential_types = [
        serde_json::json!({"email": 7, "password": "Valid-password-123!"}),
        serde_json::json!({"email": "invalid@example.test", "password": 7}),
        serde_json::json!({"email": "invalid@example.test"}),
    ];
    for (index, invalid_body) in invalid_credential_types.into_iter().enumerate() {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/register"))
            .header(header::ORIGIN, &origin)
            .json(&invalid_body)
            .send()
            .await
            .expect("invalid credentials DTO receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong credentials DTO type/shape case {index} must be rejected"
        );
    }
    let invalid_email = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .json(&serde_json::json!({
            "email": format!("{}@example.test", "a".repeat(65)),
            "password": "Valid-password-123!"
        }))
        .send()
        .await
        .expect("overlong email is rejected at the HTTP boundary");
    assert_eq!(invalid_email.status(), StatusCode::BAD_REQUEST);
    let invalid_passwords = vec!["Short-1!".to_owned(), format!("{}A1!", "a".repeat(126))];
    for invalid_password in invalid_passwords {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/register"))
            .header(header::ORIGIN, &origin)
            .json(&serde_json::json!({
                "email": format!("invalid_password_{}@example.test", Uuid::new_v4().simple()),
                "password": invalid_password
            }))
            .send()
            .await
            .expect("out-of-range password receives an HTTP response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let identities_after_invalid_credentials: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.tenant_users")
            .fetch_one(&migration_pool)
            .await
            .expect("identity count after invalid credentials is queryable");
    assert_eq!(
        identities_after_invalid_credentials, identities_before_invalid_credentials,
        "invalid credential values and types must not create identities"
    );

    let weak_password_email = format!("weak_password_{}@example.test", Uuid::new_v4().simple());
    let weak_password_response = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_weak_password")
        .json(&serde_json::json!({
            "email": weak_password_email,
            "password": "long-lowercase-password"
        }))
        .send()
        .await
        .expect("backend validates password complexity during account registration");
    assert_eq!(weak_password_response.status(), StatusCode::BAD_REQUEST);
    let weak_password_envelope: serde_json::Value = weak_password_response
        .json()
        .await
        .expect("invalid password returns a safe error envelope");
    assert_eq!(
        weak_password_envelope["error"]["correlation_id"],
        "corr_identity_weak_password"
    );
}
