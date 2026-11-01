use super::{api_key_authorization, api_key_creation, api_key_dto_validation, api_key_lifecycle, phase3_catalog, phase3_entitlements, tenant_role_matrix};
use platform_contracts::identity_access::identity_access_client::IdentityAccessClient;
use reqwest::Client;
use sqlx::PgPool;
use std::net::SocketAddr;
use tonic::transport::Channel;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    authorization_client: &mut IdentityAccessClient<Channel>,
    http_addr: SocketAddr,
    grpc_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
    created_tenant_id: Uuid,
    registered_user_id: Uuid,
) {
    api_key_dto_validation::verify(
        client,
        migration_pool,
        http_addr,
        origin,
        session_cookie,
        csrf_cookie,
        operator_session_cookie,
        operator_csrf_cookie,
        created_tenant_id,
    )
    .await;

    let (api_key_id, api_key_secret, api_key_created) = api_key_creation::verify(
        client,
        migration_pool,
        http_addr,
        origin,
        session_cookie,
        csrf_cookie,
        created_tenant_id,
    )
    .await;

    api_key_authorization::verify_allowed(
        authorization_client,
        created_tenant_id,
        api_key_id,
        &api_key_secret,
    )
    .await;
    phase3_catalog::configure(
        client,
        http_addr,
        origin,
        operator_session_cookie,
        operator_csrf_cookie,
        created_tenant_id,
    )
    .await;
    phase3_entitlements::verify(phase3_entitlements::EntitlementAcceptanceContext {
        client,
        http_addr,
        grpc_addr,
        origin,
        session_cookie,
        csrf_cookie,
        tenant_id: created_tenant_id,
        api_key_id,
        api_key_secret: &api_key_secret,
        authorization_client: authorization_client.clone(),
    })
    .await;
    api_key_authorization::verify_denials(
        authorization_client,
        migration_pool,
        created_tenant_id,
        api_key_id,
        &api_key_secret,
    )
    .await;
    api_key_lifecycle::verify(
        client,
        authorization_client,
        migration_pool,
        http_addr,
        origin,
        session_cookie,
        csrf_cookie,
        created_tenant_id,
        api_key_id,
        &api_key_secret,
        &api_key_created,
    )
    .await;

    tenant_role_matrix::verify(tenant_role_matrix::TenantRoleMatrixContext {
        client,
        migration_pool,
        authorization_client,
        http_addr,
        session_cookie,
        csrf_cookie,
        origin,
        created_tenant_id,
        registered_user_id,
    })
    .await;
}
