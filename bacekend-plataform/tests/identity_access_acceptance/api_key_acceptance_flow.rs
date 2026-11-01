pub struct ApiKeyAcceptanceContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub authorization_client: &'a mut IdentityAccessClient<Channel>,
    pub http_addr: SocketAddr,
    pub grpc_addr: SocketAddr,
    pub origin: &'a str,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
    pub registered_user_id: Uuid,
}

pub async fn verify(context: ApiKeyAcceptanceContext<'_>) {
    let ApiKeyAcceptanceContext {
        client, migration_pool, authorization_client, http_addr, grpc_addr,
        origin, session_cookie, csrf_cookie, operator_session_cookie,
        operator_csrf_cookie, created_tenant_id, registered_user_id,
    } = context;
    api_key_dto_validation::verify(api_key_dto_validation::ApiKeyDtoValidationContext {
        client, migration_pool, http_addr, origin, session_cookie, csrf_cookie,
        operator_session_cookie, operator_csrf_cookie, created_tenant_id,
    })
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
    api_key_lifecycle::verify(api_key_lifecycle::ApiKeyLifecycleContext {
        client, authorization_client, migration_pool, http_addr, origin,
        session_cookie, csrf_cookie, created_tenant_id, api_key_id,
        api_key_secret: &api_key_secret, api_key_created: &api_key_created,
    })
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
