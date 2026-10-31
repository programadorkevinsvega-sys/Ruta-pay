use crate::application::identity_service::{IdentityError, IdentityService};
use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
use payvia_observability::{current_span_id, current_trace_id};
use platform_contracts::identity_access::{
    ApiKeyMetadata, AuthorizeApiKeyOperationRequest, AuthorizeApiKeyOperationResponse,
    AuthorizeOperatorOperationRequest, AuthorizeOperatorOperationResponse,
    AuthorizeTenantOperationRequest, AuthorizeTenantOperationResponse, CreateApiKeyRequest,
    CreateApiKeyResponse, CreateOperatorSessionRequest, CreateOperatorSessionResponse,
    CreateTenantSessionRequest, CreateTenantSessionResponse, ListApiKeysRequest,
    ListApiKeysResponse, ListTenantMembershipsRequest, ListTenantMembershipsResponse,
    RegisterTenantUserRequest, RegisterTenantUserResponse, RevokeApiKeyRequest,
    RevokeApiKeyResponse, RevokeOperatorSessionRequest, RevokeOperatorSessionResponse,
    RevokeSessionRequest, RevokeSessionResponse, TenantMembership, ValidateOperatorSessionRequest,
    ValidateOperatorSessionResponse, ValidateTenantSessionRequest, ValidateTenantSessionResponse,
    identity_access_server::IdentityAccess,
};
use tonic::{Request, Response, Status};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Clone)]
pub struct IdentityAccessApi {
    service: IdentityService,
}

impl IdentityAccessApi {
    pub fn new(service: IdentityService) -> Self {
        Self { service }
    }
}

#[tonic::async_trait]
impl IdentityAccess for IdentityAccessApi {
    async fn register_tenant_user(
        &self,
        request: Request<RegisterTenantUserRequest>,
    ) -> Result<Response<RegisterTenantUserResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "register_tenant_user", &correlation_id);
        let traceparent = request
            .metadata()
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            tracing::info!(event.name = "grpc.module.request.started", component = "identity_access", operation = "register_tenant_user", correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
            match self
                .service
                .register_tenant_user(
                    &body.email,
                    &body.password,
                    &body.source_ip,
                    &correlation_id,
                    traceparent.as_deref(),
                )
                .await
            {
                Ok(user_id) => {
                    metrics::counter!("payvia_grpc_requests_total", "method" => "RegisterTenantUser", "status" => "ok").increment(1);
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "register_tenant_user", result = "success", duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(RegisterTenantUserResponse { user_id: user_id.to_string() }))
                }
                Err(error) => Err(log_and_map(error, "register_tenant_user", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    async fn create_tenant_session(
        &self,
        request: Request<CreateTenantSessionRequest>,
    ) -> Result<Response<CreateTenantSessionResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "create_tenant_session", &correlation_id);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            tracing::info!(event.name = "grpc.module.request.started", component = "identity_access", operation = "create_tenant_session", correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
            match self.service.create_tenant_session(&body.email, &body.password, &body.source_ip).await {
                Ok(session) => {
                    let response = CreateTenantSessionResponse {
                        session_token: session.session_token.expose_once().to_owned(),
                        csrf_token: session.csrf_token.expose_once().to_owned(),
                        expires_at: session.expires_at.to_rfc3339(),
                        user_id: session.user_id.to_string(),
                    };
                    metrics::counter!("payvia_grpc_requests_total", "method" => "CreateTenantSession", "status" => "ok").increment(1);
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "create_tenant_session", result = "success", duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(response))
                }
                Err(error) => Err(log_and_map(error, "create_tenant_session", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    async fn revoke_session(
        &self,
        request: Request<RevokeSessionRequest>,
    ) -> Result<Response<RevokeSessionResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "revoke_session", &correlation_id);
        let traceparent = request
            .metadata()
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            match self
                .service
                .revoke_tenant_session(
                    &body.session_token,
                    &correlation_id,
                    traceparent.as_deref(),
                )
                .await
            {
                Ok(()) => {
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "revoke_session", result = "success", duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(RevokeSessionResponse {}))
                }
                Err(error) => Err(log_and_map(error, "revoke_session", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    async fn validate_tenant_session(
        &self,
        request: Request<ValidateTenantSessionRequest>,
    ) -> Result<Response<ValidateTenantSessionResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "validate_tenant_session", &correlation_id);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            match self.service.validate_tenant_session(&body.session_token, &body.csrf_token).await {
                Ok(user_id) => {
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "validate_tenant_session", result = "success", duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(ValidateTenantSessionResponse { user_id: user_id.to_string() }))
                }
                Err(error) => Err(log_and_map(error, "validate_tenant_session", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    async fn create_api_key(
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

    async fn list_api_keys(
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

    async fn revoke_api_key(
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

    async fn authorize_api_key_operation(
        &self,
        request: Request<AuthorizeApiKeyOperationRequest>,
    ) -> Result<Response<AuthorizeApiKeyOperationResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "authorize_api_key_operation", &correlation_id);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id, "tenant identifier")?;
        async move {
            let started = std::time::Instant::now();
            match self.service.authorize_api_key_operation(
                &body.secret,
                tenant_id,
                &body.environment,
                &body.required_scope,
                &correlation_id,
            ).await {
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

    async fn create_operator_session(
        &self,
        request: Request<CreateOperatorSessionRequest>,
    ) -> Result<Response<CreateOperatorSessionResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "create_operator_session", &correlation_id);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            tracing::info!(event.name = "grpc.module.request.started", component = "identity_access", operation = "create_operator_session", correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
            match self
                .service
                .create_operator_session(&body.email, &body.password, &body.source_ip)
                .await
            {
                Ok(session) => {
                    let response = CreateOperatorSessionResponse {
                        user_id: session.user_id.to_string(),
                        session_token: session.session_token.expose_once().to_owned(),
                        csrf_token: session.csrf_token.expose_once().to_owned(),
                        expires_at: session.expires_at.to_rfc3339(),
                    };
                    tracing::info!(event.name = "grpc.module.response", component = "identity_access", operation = "create_operator_session", result = "success", duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(response))
                }
                Err(error) => Err(log_and_map(
                    error,
                    "create_operator_session",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )),
            }
        }
        .instrument(span)
        .await
    }

    async fn revoke_operator_session(
        &self,
        request: Request<RevokeOperatorSessionRequest>,
    ) -> Result<Response<RevokeOperatorSessionResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "revoke_operator_session", &correlation_id);
        let traceparent = request
            .metadata()
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            match self
                .service
                .revoke_operator_session(
                    &body.session_token,
                    &correlation_id,
                    traceparent.as_deref(),
                )
                .await
            {
                Ok(()) => Ok(Response::new(RevokeOperatorSessionResponse {})),
                Err(error) => Err(log_and_map(
                    error,
                    "revoke_operator_session",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )),
            }
        }
        .instrument(span)
        .await
    }

    async fn validate_operator_session(
        &self,
        request: Request<ValidateOperatorSessionRequest>,
    ) -> Result<Response<ValidateOperatorSessionResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "validate_operator_session", &correlation_id);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            match self
                .service
                .validate_operator_session(&body.session_token, &body.csrf_token)
                .await
            {
                Ok(user_id) => Ok(Response::new(ValidateOperatorSessionResponse {
                    user_id: user_id.to_string(),
                })),
                Err(error) => Err(log_and_map(
                    error,
                    "validate_operator_session",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )),
            }
        }
        .instrument(span)
        .await
    }

    async fn authorize_operator_operation(
        &self,
        request: Request<AuthorizeOperatorOperationRequest>,
    ) -> Result<Response<AuthorizeOperatorOperationResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "authorize_operator_operation", &correlation_id);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            match self
                .service
                .authorize_operator_operation(
                    &body.session_token,
                    &body.csrf_token,
                    &body.permission,
                    &correlation_id,
                )
                .await
            {
                Ok(operator_id) => Ok(Response::new(AuthorizeOperatorOperationResponse {
                    operator_id: operator_id.to_string(),
                })),
                Err(error) => Err(log_and_map(
                    error,
                    "authorize_operator_operation",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )),
            }
        }
        .instrument(span)
        .await
    }

    async fn authorize_tenant_operation(
        &self,
        request: Request<AuthorizeTenantOperationRequest>,
    ) -> Result<Response<AuthorizeTenantOperationResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "authorize_tenant_operation", &correlation_id);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id, "tenant identifier")?;
        async move {
            let started = std::time::Instant::now();
            match self
                .service
                .authorize_tenant_operation(
                    &body.session_token,
                    &body.csrf_token,
                    tenant_id,
                    &body.permission,
                    &correlation_id,
                )
                .await
            {
                Ok(user_id) => Ok(Response::new(AuthorizeTenantOperationResponse {
                    user_id: user_id.to_string(),
                })),
                Err(error) => Err(log_and_map(
                    error,
                    "authorize_tenant_operation",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )),
            }
        }
        .instrument(span)
        .await
    }

    async fn list_tenant_memberships(
        &self,
        request: Request<ListTenantMembershipsRequest>,
    ) -> Result<Response<ListTenantMembershipsResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "list_tenant_memberships", &correlation_id);
        let body = request.into_inner();
        async move {
            let started = std::time::Instant::now();
            match self
                .service
                .list_tenant_memberships(&body.session_token, &body.csrf_token)
                .await
            {
                Ok((_, memberships)) => Ok(Response::new(ListTenantMembershipsResponse {
                    memberships: memberships
                        .into_iter()
                        .map(|membership| TenantMembership {
                            tenant_id: membership.tenant_id.to_string(),
                            role: membership.role,
                            membership_status: membership.membership_status,
                        })
                        .collect(),
                })),
                Err(error) => Err(log_and_map(
                    error,
                    "list_tenant_memberships",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )),
            }
        }
        .instrument(span)
        .await
    }
}

fn grpc_span<T>(
    request: &Request<T>,
    operation: &'static str,
    correlation_id: &str,
) -> tracing::Span {
    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&GrpcMetadataExtractor(request.metadata()))
    });
    let parent_trace_id = parent.span().span_context().trace_id().to_string();
    let span = tracing::info_span!(
        "grpc.module.identity_access",
        operation,
        correlation_id,
        trace_id = tracing::field::Empty
    );
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(event.name = "grpc.trace_context.rejected", component = "identity_access", operation, error_category = "trace_context", error_code = %error, correlation_id, trace_id = %parent_trace_id);
    }
    span
}

struct GrpcMetadataExtractor<'a>(&'a tonic::metadata::MetadataMap);

impl Extractor for GrpcMetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|key| match key {
                tonic::metadata::KeyRef::Ascii(value) => Some(value.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}

fn parse_uuid(value: &str, field: &'static str) -> Result<uuid::Uuid, Status> {
    uuid::Uuid::parse_str(value).map_err(|_| {
        tracing::warn!(
            event.name = "grpc.module.request.failed",
            component = "identity_access",
            operation = "parse_identifier",
            error_code = "identifier_invalid",
            error_category = "validation",
            field
        );
        Status::invalid_argument("identifier is invalid")
    })
}

fn log_and_map(
    error: IdentityError,
    operation: &'static str,
    correlation_id: &str,
    duration_ms: u64,
) -> Status {
    let (code, category, status) = match &error {
        IdentityError::Email(_)
        | IdentityError::Password(_)
        | IdentityError::ApiKeyInputInvalid => (
            "identity_input_invalid",
            "validation",
            Status::invalid_argument("identity input is invalid"),
        ),
        IdentityError::AccountConflict | IdentityError::OperatorBootstrapClosed => (
            "identity_conflict",
            "conflict",
            Status::already_exists("identity resource already exists"),
        ),
        IdentityError::AuthenticationFailed => (
            "authentication_failed",
            "authentication",
            Status::unauthenticated("authentication failed"),
        ),
        IdentityError::RateLimited => (
            "identity_rate_limited",
            "rate_limit",
            Status::resource_exhausted("login rate limit exceeded"),
        ),
        IdentityError::Redis(_) => {
            tracing::error!(
                event.name = "grpc.dependency.call.failed",
                component = "identity_access",
                operation,
                dependency = "redis",
                error_code = "rate_limit_dependency_unavailable",
                error_category = "connectivity",
                dependency_duration_ms = duration_ms,
                correlation_id,
                trace_id = %current_trace_id(),
                span_id = %current_span_id(),
                retryable = true
            );
            (
                "rate_limit_unavailable",
                "dependency",
                Status::unavailable("identity operation could not be completed"),
            )
        }
        IdentityError::SourceAddressInvalid => (
            "identity_input_invalid",
            "validation",
            Status::invalid_argument("identity input is invalid"),
        ),
        IdentityError::AuthorizationDenied => (
            "authorization_denied",
            "authorization",
            Status::permission_denied("operation is not permitted"),
        ),
        IdentityError::ApiKeyNotFound => (
            "api_key_not_found",
            "not_found",
            Status::not_found("API key was not found"),
        ),
        IdentityError::Database(sqlx_error) => {
            let state = sqlx_error
                .as_database_error()
                .and_then(|database| database.code())
                .unwrap_or_default();
            tracing::error!(event.name = "grpc.dependency.call.failed", component = "identity_access", operation, dependency = "postgresql", error_code = %state, error_category = "persistence", dependency_duration_ms = duration_ms, correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id(), retryable = false);
            (
                "identity_persistence_failed",
                "persistence",
                Status::internal("identity operation could not be completed"),
            )
        }
        IdentityError::PasswordHash(_)
        | IdentityError::HashingTaskFailed
        | IdentityError::RandomSourceUnavailable
        | IdentityError::TimeConfigurationInvalid => (
            "identity_internal_failure",
            "internal",
            Status::internal("identity operation could not be completed"),
        ),
    };
    tracing::warn!(
        event.name = "grpc.module.request.failed",
        component = "identity_access",
        operation,
        error_code = code,
        error_category = category,
        duration_total_ms = duration_ms,
        correlation_id,
        trace_id = %current_trace_id(),
        span_id = %current_span_id(),
        retryable = false
    );
    metrics::counter!("payvia_grpc_requests_total", "method" => operation, "status" => code)
        .increment(1);
    status
}
