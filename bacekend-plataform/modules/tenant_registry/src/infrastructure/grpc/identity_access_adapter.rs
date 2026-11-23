use async_trait::async_trait;
use opentelemetry::propagation::Injector;
use platform_contracts::identity_access::{
    AuthorizeOperatorOperationRequest, AuthorizeTenantOperationRequest,
    ValidateTenantSessionRequest, identity_access_client::IdentityAccessClient as GrpcClient,
};
use std::time::{Duration, Instant};
use tonic::{
    Request, Status,
    metadata::{MetadataKey, MetadataMap, MetadataValue},
    transport::Channel,
};
use uuid::Uuid;

use crate::application::identity_access_port::{IdentityAccessError, IdentityAccessPort};

#[derive(Clone)]
pub struct IdentityAccessGrpcAdapter {
    client: GrpcClient<Channel>,
}

impl IdentityAccessGrpcAdapter {
    pub fn new(client: GrpcClient<Channel>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl IdentityAccessPort for IdentityAccessGrpcAdapter {
    async fn validate_tenant_session(
        &self,
        token: &str,
        csrf_token: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError> {
        let request = contextual_request(
            ValidateTenantSessionRequest {
                session_token: token.to_owned(),
                csrf_token: csrf_token.to_owned(),
                correlation_id: correlation_id.to_owned(),
            },
            correlation_id,
        )?;
        let started = dependency_started("validate_tenant_session", correlation_id);
        let response = self
            .client
            .clone()
            .validate_tenant_session(request)
            .await
            .map_err(|status| {
                record_dependency_failure(
                    status,
                    "validate_tenant_session",
                    correlation_id,
                    started.elapsed(),
                )
            })?
            .into_inner();
        record_dependency_success("validate_tenant_session", correlation_id, started.elapsed());
        Uuid::parse_str(&response.user_id).map_err(|_| IdentityAccessError::Unavailable)
    }

    async fn authorize_tenant(
        &self,
        token: &str,
        csrf_token: &str,
        tenant_id: Uuid,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError> {
        let request = contextual_request(
            AuthorizeTenantOperationRequest {
                session_token: token.to_owned(),
                csrf_token: csrf_token.to_owned(),
                tenant_id: tenant_id.to_string(),
                permission: permission.to_owned(),
                correlation_id: correlation_id.to_owned(),
            },
            correlation_id,
        )?;
        let started = dependency_started("authorize_tenant_operation", correlation_id);
        let response = self
            .client
            .clone()
            .authorize_tenant_operation(request)
            .await
            .map_err(|status| {
                record_dependency_failure(
                    status,
                    "authorize_tenant_operation",
                    correlation_id,
                    started.elapsed(),
                )
            })?
            .into_inner();
        record_dependency_success(
            "authorize_tenant_operation",
            correlation_id,
            started.elapsed(),
        );
        Uuid::parse_str(&response.user_id).map_err(|_| IdentityAccessError::Unavailable)
    }

    async fn authorize_operator(
        &self,
        token: &str,
        csrf_token: &str,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError> {
        let request = contextual_request(
            AuthorizeOperatorOperationRequest {
                session_token: token.to_owned(),
                csrf_token: csrf_token.to_owned(),
                permission: permission.to_owned(),
                correlation_id: correlation_id.to_owned(),
            },
            correlation_id,
        )?;
        let started = dependency_started("authorize_operator_operation", correlation_id);
        let response = self
            .client
            .clone()
            .authorize_operator_operation(request)
            .await
            .map_err(|status| {
                record_dependency_failure(
                    status,
                    "authorize_operator_operation",
                    correlation_id,
                    started.elapsed(),
                )
            })?
            .into_inner();
        record_dependency_success(
            "authorize_operator_operation",
            correlation_id,
            started.elapsed(),
        );
        Uuid::parse_str(&response.operator_id).map_err(|_| IdentityAccessError::Unavailable)
    }
}

fn contextual_request<T>(body: T, correlation_id: &str) -> Result<Request<T>, IdentityAccessError> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "x-correlation-id",
        MetadataValue::try_from(correlation_id).map_err(|_| IdentityAccessError::Invalid)?,
    );
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject(&mut GrpcMetadataInjector(request.metadata_mut()))
    });
    request.set_timeout(Duration::from_secs(5));
    Ok(request)
}

struct GrpcMetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for GrpcMetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        let Ok(key) = MetadataKey::from_bytes(key.as_bytes()) else {
            tracing::warn!(
                event.name = "grpc.trace_context.inject.failed",
                component = "tenant_registry",
                operation = "identity_access_request",
                error_code = "trace_context_metadata_invalid"
            );
            return;
        };
        let Ok(value) = MetadataValue::try_from(value) else {
            tracing::warn!(
                event.name = "grpc.trace_context.inject.failed",
                component = "tenant_registry",
                operation = "identity_access_request",
                error_code = "trace_context_metadata_invalid"
            );
            return;
        };
        self.0.insert(key, value);
    }
}

fn dependency_started(operation: &'static str, correlation_id: &str) -> Instant {
    tracing::info!(
        event.name = "grpc.dependency.call.started",
        component = "tenant_registry",
        operation,
        dependency = "identity_access",
        correlation_id,
        trace_id = %payvia_observability::current_trace_id(),
        span_id = %payvia_observability::current_span_id()
    );
    Instant::now()
}

fn record_dependency_success(
    operation: &'static str,
    correlation_id: &str,
    duration: std::time::Duration,
) {
    tracing::info!(
        event.name = "grpc.dependency.call.completed",
        component = "tenant_registry",
        operation,
        dependency = "identity_access",
        result = "success",
        dependency_duration_ms = duration.as_millis() as u64,
        correlation_id,
        trace_id = %payvia_observability::current_trace_id(),
        span_id = %payvia_observability::current_span_id()
    );
}

fn record_dependency_failure(
    status: Status,
    operation: &'static str,
    correlation_id: &str,
    duration: std::time::Duration,
) -> IdentityAccessError {
    let (error_code, error_category) = match status.code() {
        tonic::Code::Unauthenticated => ("authentication_failed", "authentication"),
        tonic::Code::PermissionDenied => ("authorization_denied", "authorization"),
        tonic::Code::InvalidArgument => ("identity_request_invalid", "validation"),
        tonic::Code::DeadlineExceeded => ("identity_dependency_timeout", "timeout"),
        tonic::Code::Unavailable => ("identity_dependency_unavailable", "dependency"),
        _ => ("identity_dependency_failed", "dependency"),
    };
    tracing::warn!(
        event.name = "grpc.dependency.call.failed",
        component = "tenant_registry",
        operation,
        dependency = "identity_access",
        grpc_code = ?status.code(),
        error_code,
        error_category,
        retryable = false,
        dependency_duration_ms = duration.as_millis() as u64,
        correlation_id,
        trace_id = %payvia_observability::current_trace_id(),
        span_id = %payvia_observability::current_span_id()
    );
    map_status(status)
}

fn map_status(status: tonic::Status) -> IdentityAccessError {
    match status.code() {
        tonic::Code::Unauthenticated => IdentityAccessError::Authentication,
        tonic::Code::PermissionDenied => IdentityAccessError::Authorization,
        tonic::Code::InvalidArgument => IdentityAccessError::Invalid,
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
            IdentityAccessError::Unavailable
        }
        _ => IdentityAccessError::Unavailable,
    }
}
