use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let operator_session_api_key_list = ctx.client
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
let operator_session_api_key_create = ctx.client
    .post(format!(
        "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
    ))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.csrf_cookie)
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
        .fetch_one(&ctx.migration_pool)
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
    let response = ctx.client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &ctx.origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &ctx.state.csrf_cookie)
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
    let response = ctx.client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &ctx.origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &ctx.state.csrf_cookie)
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
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("API key count after invalid DTO checks is queryable");
assert_eq!(
    api_keys_after_invalid_dtos, api_keys_before_invalid_dtos,
    "invalid API key values and types must not persist API keys"
);
 sqlx::raw_sql(
    "CREATE OR REPLACE FUNCTION identity_access.reject_test_api_key_outbox_event() RETURNS trigger \
     LANGUAGE plpgsql AS $api_key$ BEGIN \
     IF NEW.correlation_id = 'corr_api_key_outbox_failure' THEN \
         RAISE EXCEPTION 'injected API key Outbox failure'; \
     END IF; RETURN NEW; END $api_key$; \
     CREATE TRIGGER reject_test_api_key_outbox_event BEFORE INSERT ON identity_access.outbox_events \
     FOR EACH ROW EXECUTE FUNCTION identity_access.reject_test_api_key_outbox_event();",
)
.execute(&ctx.migration_pool)
.await
.expect("API key Outbox failure trigger is installed");
let api_keys_before_outbox_failure: i64 =
    sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("API key count before Outbox failure is queryable");}
