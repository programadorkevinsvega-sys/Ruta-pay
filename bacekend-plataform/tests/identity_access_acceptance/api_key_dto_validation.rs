use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
    created_tenant_id: Uuid,
) {
    let operator_session_api_key_list = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .send()
        .await
        .expect("operator session cannot use tenant API key endpoints");
    assert_eq!(
        operator_session_api_key_list.status(),
        StatusCode::UNAUTHORIZED
    );
    let operator_session_api_key_create = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .json(&serde_json::json!({
            "name": "Wrong actor",
            "environment": "test",
            "scopes": ["payment_intents.create", "usage.consume"]
        }))
        .send()
        .await
        .expect("operator session cannot create tenant API keys");
    assert_eq!(
        operator_session_api_key_create.status(),
        StatusCode::UNAUTHORIZED
    );

    let api_keys_before_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count before invalid DTO checks is queryable");
    let invalid_api_key_values = [
        serde_json::json!({"name": "x", "environment": "test", "scopes": ["read"]}),
        serde_json::json!({"name": "x".repeat(81), "environment": "test", "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": "production", "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": []}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": ["Read"]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": ["a".repeat(65)]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": vec!["read"; 17]}),
    ];
    for (index, invalid_body) in invalid_api_key_values.into_iter().enumerate() {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("semantically invalid API key DTO receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "invalid API key DTO case {index} must fail validation"
        );
    }
    let invalid_api_key_types = [
        serde_json::json!({"name": 7, "environment": "test", "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": 7, "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": "read"}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": [7]}),
    ];
    for (index, invalid_body) in invalid_api_key_types.into_iter().enumerate() {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("incorrect API key DTO field type receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong API key DTO type case {index} must be rejected by the HTTP extractor"
        );
    }
    let api_keys_after_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count after invalid DTO checks is queryable");
    assert_eq!(
        api_keys_after_invalid_dtos, api_keys_before_invalid_dtos,
        "invalid API key values and types must not persist API keys"
    );
}
