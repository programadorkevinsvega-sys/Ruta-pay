use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};
use uuid::Uuid;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let api_key_creation = ctx.client
    .post(format!(
        "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
    ))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.csrf_cookie)
    .header("x-correlation-id", "corr_api_key_create")
    .json(&serde_json::json!({
        "name": "Acceptance live key",
        "environment": "live",
        "scopes": ["payment_intents.create", "usage.consume"]
    }))
    .send()
    .await
    .expect(
        "tenant owner can create an API key with payment and quota scopes through BFF and gRPC",
    );
assert_eq!(api_key_creation.status(), StatusCode::CREATED);
let api_key_created: serde_json::Value = api_key_creation
    .json()
    .await
    .expect("API key creation returns the one-time secret");
ctx.state.ctx.state.api_key_id = Uuid::parse_str(
    api_key_created["key_id"]
        .as_str()
        .expect("API key id is present"),
)
.expect("API key id is a UUID");
ctx.state.ctx.state.api_key_secret = api_key_created["secret_once"]
    .as_str()
    .expect("one-time API key secret is present");
assert!(ctx.state.api_key_secret.starts_with("live_"));
assert_eq!(
    api_key_created["prefix"].as_str(),
    Some(&ctx.state.api_key_secret[..13])
);
assert_eq!(api_key_created["environment"], "live");
let stored_key_digest: Vec<u8> = sqlx::query_scalar(
    "SELECT secret_digest FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
)
.bind(ctx.state.created_tenant_id)
.bind(ctx.state.api_key_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("API key digest is persisted");
assert_eq!(
    stored_key_digest.as_slice(),
    Sha256::digest(ctx.state.api_key_secret.as_bytes()).as_slice()
);
 let scoped_api_key_authorization = ctx.authorization_client
    .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
        secret: ctx.state.api_key_secret.to_owned(),
        tenant_id: ctx.state.created_tenant_id.to_string(),
        environment: "live".to_owned(),
        required_scope: "payment_intents.create".to_owned(),
        correlation_id: "corr_api_key_scope_allowed".to_owned(),
    })
    .await
    .expect("API key with the requested scope authorizes the operation")
    .into_inner();
assert_eq!(scoped_api_key_authorization.key_id, ctx.state.api_key_id.to_string());
assert_eq!(
    scoped_api_key_authorization.tenant_id,
    ctx.state.created_tenant_id.to_string()
);
phase3_catalog::configure(
    &ctx.client,
    ctx.http_addr,
    &ctx.origin,
    &operator_session_cookie,
    &operator_csrf_cookie,
    ctx.state.created_tenant_id,
)
.await;
phase3_entitlements::verify(
    &ctx.client,
    ctx.http_addr,
    grpc_addr,
    &ctx.origin,
    &ctx.state.session_cookie,
    &ctx.state.csrf_cookie,
    ctx.state.created_tenant_id,
    ctx.state.api_key_id,
    &ctx.state.api_key_secret,
    ctx.authorization_client.clone(),
)
.await;
 let denied_api_key_scope = ctx.authorization_client
    .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
        secret: ctx.state.api_key_secret.to_owned(),
        tenant_id: ctx.state.created_tenant_id.to_string(),
        environment: "live".to_owned(),
        required_scope: "payment_intents.capture".to_owned(),
        correlation_id: "corr_api_key_scope_denied".to_owned(),
    })
    .await
    .expect_err("API key cannot authorize an operation outside its scopes");
assert_eq!(denied_api_key_scope.code(), tonic::Code::PermissionDenied);
let wrong_environment_api_key = ctx.authorization_client
    .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
        secret: ctx.state.api_key_secret.to_owned(),
        tenant_id: ctx.state.created_tenant_id.to_string(),
        environment: "test".to_owned(),
        required_scope: "payment_intents.create".to_owned(),
        correlation_id: "corr_api_key_wrong_environment".to_owned(),
    })
    .await
    .expect_err("live API key cannot authorize test environment operations");
assert_eq!(
    wrong_environment_api_key.code(),
    tonic::Code::Unauthenticated
);
let cross_tenant_api_key = ctx.authorization_client
    .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
        secret: ctx.state.api_key_secret.to_owned(),
        tenant_id: Uuid::new_v4().to_string(),
        environment: "live".to_owned(),
        required_scope: "payment_intents.create".to_owned(),
        correlation_id: "corr_api_key_cross_tenant".to_owned(),
    })
    .await
    .expect_err("API key cannot authorize a different tenant");
assert_eq!(cross_tenant_api_key.code(), tonic::Code::Unauthenticated);
let api_key_last_used: bool = sqlx::query_scalar(
    "SELECT last_used_at IS NOT NULL FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
)
.bind(ctx.state.created_tenant_id)
.bind(ctx.state.api_key_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("successful scoped authorization updates last_used_at");
assert!(api_key_last_used);
let api_key_scope_audit_counts: (i64, i64) = sqlx::query_as(
    "SELECT count(*) FILTER (WHERE outcome='allowed'),count(*) FILTER (WHERE outcome='denied') FROM identity_access.audit_events WHERE action='api_key.authorization.checked' AND resource_id=$1 AND correlation_id IN ('corr_api_key_scope_allowed','corr_api_key_scope_denied')",
)
.bind(ctx.state.api_key_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("API key scope authorization audit outcomes are queryable");
assert_eq!(api_key_scope_audit_counts, (1, 1));
}
