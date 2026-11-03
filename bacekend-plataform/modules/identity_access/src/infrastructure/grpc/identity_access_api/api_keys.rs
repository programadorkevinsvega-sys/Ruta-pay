use super::support::{grpc_span, log_and_map, parse_uuid};
use super::*;

impl IdentityAccessApi {
    pub(super) async fn create_api_key(
        &self,
        request: Request<CreateApiKeyRequest>,
    ) -> Result<Response<CreateApiKeyResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "create_api_key", &correlation_id);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id, "tenant identifier")?;
        async move {
            let started = std::time::Instant::now();
            match self.service.create_api_key(&body.session_token, tenant_id, &body.name, &body.environment, &body.scopes, &correlation_id).await {
                Ok(created) => {
                    let result = CreateApiKeyResponse {
                        key_id: created.key.id.to_string(),
                        secret_once: created.key.secret,
                        prefix: created.key.prefix,
                        environment: created.key.environment.as_str().to_owned(),
                        scopes: created.key.scopes,
                        created_at: chrono::Utc::now().to_rfc3339(),
                    };
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "create_api_key", result = "success", tenant_id = %tenant_id, duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(result))
                }
                Err(error) => Err(log_and_map(error, "create_api_key", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    pub(super) async fn list_api_keys(
        &self,
        request: Request<ListApiKeysRequest>,
    ) -> Result<Response<ListApiKeysResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "list_api_keys", &correlation_id);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id, "tenant identifier")?;
        async move {
            let started = std::time::Instant::now();
            match self.service.list_api_keys(&body.session_token, tenant_id).await {
                Ok(keys) => {
                    let api_keys = keys.into_iter().map(|key| ApiKeyMetadata {
                        key_id: key.id.to_string(), name: key.name, prefix: key.prefix,
                        environment: key.environment, scopes: key.scopes, status: key.status,
                        created_at: key.created_at.to_rfc3339(),
                        revoked_at: key.revoked_at.map(|value| value.to_rfc3339()).unwrap_or_default(),
                    }).collect();
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "list_api_keys", result = "success", tenant_id = %tenant_id, duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(ListApiKeysResponse { api_keys }))
                }
                Err(error) => Err(log_and_map(error, "list_api_keys", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    pub(super) async fn revoke_api_key(
        &self,
        request: Request<RevokeApiKeyRequest>,
    ) -> Result<Response<RevokeApiKeyResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "revoke_api_key", &correlation_id);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id, "tenant identifier")?;
        let key_id = parse_uuid(&body.key_id, "API key identifier")?;
        async move {
            let started = std::time::Instant::now();
            match self.service.revoke_api_key(&body.session_token, tenant_id, key_id, &correlation_id).await {
                Ok(()) => {
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "revoke_api_key", result = "success", tenant_id = %tenant_id, duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(RevokeApiKeyResponse {}))
                }
                Err(error) => Err(log_and_map(error, "revoke_api_key", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    pub(super) async fn authorize_api_key_operation(
        &self,
        request: Request<AuthorizeApiKeyOperationRequest>,
    ) -> Result<Response<AuthorizeApiKeyOperationResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "authorize_api_key_operation", &correlation_id);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id, "tenant identifier")?;
        async move {
            let started = std::time::Instant::now();
            match self
                .service
                .authorize_api_key_operation(
                    &body.secret,
                    tenant_id,
                    &body.environment,
                    &body.required_scope,
                    &correlation_id,
                )
                .await
            {
                Ok(authorization) => {
                    tracing::info!(
                        event.name = "grpc.module.response",
                        component = "identity_access",
                        operation = "authorize_api_key_operation",
                        result = "success",
                        tenant_id = %authorization.tenant_id,
                        key_id = %authorization.key_id,
                        environment = %authorization.environment,
                        duration_total_ms = started.elapsed().as_millis() as u64,
                        correlation_id = %correlation_id,
                        trace_id = %current_trace_id(),
                        span_id = %current_span_id()
                    );
                    Ok(Response::new(AuthorizeApiKeyOperationResponse {
                        key_id: authorization.key_id.to_string(),
                        tenant_id: authorization.tenant_id.to_string(),
                        environment: authorization.environment,
                    }))
                }
                Err(error) => Err(log_and_map(
                    error,
                    "authorize_api_key_operation",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )),
            }
        }
        .instrument(span)
        .await
    }
}
