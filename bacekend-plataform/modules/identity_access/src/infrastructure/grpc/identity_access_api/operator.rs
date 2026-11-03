use super::support::{grpc_span, log_and_map};
use super::*;

impl IdentityAccessApi {
    pub(super) async fn create_operator_session(
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

    pub(super) async fn revoke_operator_session(
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

    pub(super) async fn validate_operator_session(
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

    pub(super) async fn authorize_operator_operation(
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
}
