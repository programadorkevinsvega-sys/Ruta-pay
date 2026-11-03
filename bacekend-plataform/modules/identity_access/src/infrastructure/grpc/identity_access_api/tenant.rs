use super::support::{grpc_span, log_and_map, parse_uuid};
use super::*;

impl IdentityAccessApi {
    pub(super) async fn register_tenant_user(
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

    pub(super) async fn create_tenant_session(
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

    pub(super) async fn revoke_session(
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

    pub(super) async fn validate_tenant_session(
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

    pub(super) async fn authorize_tenant_operation(
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

    pub(super) async fn list_tenant_memberships(
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
