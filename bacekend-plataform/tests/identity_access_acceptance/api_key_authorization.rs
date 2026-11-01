use platform_contracts::identity_access::{
    AuthorizeApiKeyOperationRequest,
    identity_access_client::IdentityAccessClient,
};
use sqlx::PgPool;
use tonic::{Code, transport::Channel};
use uuid::Uuid;

pub async fn verify_allowed(
    authorization_client: &mut IdentityAccessClient<Channel>,
    created_tenant_id: Uuid,
    api_key_id: Uuid,
    api_key_secret: &str,
) {
    let authorization = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_scope_allowed".to_owned(),
        })
        .await
        .expect("API key with the requested scope authorizes the operation")
        .into_inner();
    assert_eq!(authorization.key_id, api_key_id.to_string());
    assert_eq!(authorization.tenant_id, created_tenant_id.to_string());
}

pub async fn verify_denials(
    authorization_client: &mut IdentityAccessClient<Channel>,
    migration_pool: &PgPool,
    created_tenant_id: Uuid,
    api_key_id: Uuid,
    api_key_secret: &str,
) {
    let denied_scope = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.capture".to_owned(),
            correlation_id: "corr_api_key_scope_denied".to_owned(),
        })
        .await
        .expect_err("API key cannot authorize an operation outside its scopes");
    assert_eq!(denied_scope.code(), Code::PermissionDenied);
    let wrong_environment = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "test".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_wrong_environment".to_owned(),
        })
        .await
        .expect_err("live API key cannot authorize test environment operations");
    assert_eq!(wrong_environment.code(), Code::Unauthenticated);
    let cross_tenant = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: Uuid::new_v4().to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_cross_tenant".to_owned(),
        })
        .await
        .expect_err("API key cannot authorize a different tenant");
    assert_eq!(cross_tenant.code(), Code::Unauthenticated);
    let last_used: bool = sqlx::query_scalar(
        "SELECT last_used_at IS NOT NULL FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
    )
    .bind(created_tenant_id)
    .bind(api_key_id)
    .fetch_one(migration_pool)
    .await
    .expect("successful scoped authorization updates last_used_at");
    assert!(last_used);
    let audit_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE outcome='allowed'),count(*) FILTER (WHERE outcome='denied') FROM identity_access.audit_events WHERE action='api_key.authorization.checked' AND resource_id=$1 AND correlation_id IN ('corr_api_key_scope_allowed','corr_api_key_scope_denied')",
    )
    .bind(api_key_id)
    .fetch_one(migration_pool)
    .await
    .expect("API key scope authorization audit outcomes are queryable");
    assert_eq!(audit_counts, (1, 1));
}
