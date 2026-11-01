use super::*;

#[utoipa::path(post, path = "/v1/tenants/{tenant_id}/api-keys", params(("tenant_id" = String, Path)), request_body = NewApiKey, responses((status = 201, body = ApiKeyCreated), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn create_api_key(
    State(state): State<std::sync::Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<NewApiKey>,
) -> Response {
    let correlation = correlation_id(&headers);
    let session_token = match validate_mutation_session(&state, &headers, &correlation).await {
        Ok(token) => token,
        Err(response) => return *response,
    };
    let started = std::time::Instant::now();
    let rpc = CreateApiKeyRequest {
        session_token,
        tenant_id: tenant_id.to_string(),
        name: body.name,
        environment: body.environment,
        scopes: body.scopes,
        correlation_id: correlation.clone(),
    };
    match IdentityAccessClient::create_api_key(&state.identity_client, rpc, &correlation).await {
        Ok(key) => (
            StatusCode::CREATED,
            Json(ApiKeyCreated {
                key_id: key.key_id,
                secret_once: key.secret_once,
                prefix: key.prefix,
                environment: key.environment,
                scopes: key.scopes,
                created_at: key.created_at,
            }),
        )
            .into_response(),
        Err(status) => grpc_error(
            status,
            "create_api_key",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path = "/v1/tenants/{tenant_id}/api-keys", params(("tenant_id" = String, Path)), responses((status = 200, body = [ApiKeyMetadataResponse]), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn list_api_keys(
    State(state): State<std::sync::Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some(session_token) = cookie_value(&headers, SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let started = std::time::Instant::now();
    let request = ListApiKeysRequest {
        session_token,
        tenant_id: tenant_id.to_string(),
        correlation_id: correlation.clone(),
    };
    match IdentityAccessClient::list_api_keys(&state.identity_client, request, &correlation).await {
        Ok(keys) => Json(
            keys.api_keys
                .into_iter()
                .map(|key| ApiKeyMetadataResponse {
                    key_id: key.key_id,
                    name: key.name,
                    prefix: key.prefix,
                    environment: key.environment,
                    scopes: key.scopes,
                    status: key.status,
                    created_at: key.created_at,
                    revoked_at: (!key.revoked_at.is_empty()).then_some(key.revoked_at),
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(status) => grpc_error(
            status,
            "list_api_keys",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(delete, path = "/v1/tenants/{tenant_id}/api-keys/{key_id}", params(("tenant_id" = String, Path), ("key_id" = String, Path)), responses((status = 204), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn revoke_api_key(
    State(state): State<std::sync::Arc<AppState>>,
    Path((tenant_id, key_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let session_token = match validate_mutation_session(&state, &headers, &correlation).await {
        Ok(token) => token,
        Err(response) => return *response,
    };
    let started = std::time::Instant::now();
    let request = RevokeApiKeyRequest {
        session_token,
        tenant_id: tenant_id.to_string(),
        key_id: key_id.to_string(),
        correlation_id: correlation.clone(),
    };
    match IdentityAccessClient::revoke_api_key(&state.identity_client, request, &correlation).await
    {
        Ok(_) => response_no_content(),
        Err(status) => grpc_error(
            status,
            "revoke_api_key",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}
