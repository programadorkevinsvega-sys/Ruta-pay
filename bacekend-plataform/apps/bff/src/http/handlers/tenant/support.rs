use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};

use crate::http::handlers::identity;

pub(super) const TENANT_SESSION_COOKIE: &str = "payvia_tenant_session";
pub(super) const TENANT_CSRF_COOKIE: &str = "payvia_tenant_csrf";
pub(super) const OPERATOR_SESSION_COOKIE: &str = "payvia_platform_session";
pub(super) const OPERATOR_CSRF_COOKIE: &str = "payvia_platform_csrf";

pub(super) fn csrf_value(headers: &HeaderMap, cookie_name: &str) -> Option<(String, String)> {
    if !identity::valid_origin(headers) || !identity::valid_csrf(headers, cookie_name) {
        return None;
    }
    let session_cookie = if cookie_name == TENANT_CSRF_COOKIE {
        TENANT_SESSION_COOKIE
    } else {
        OPERATOR_SESSION_COOKIE
    };
    Some((
        identity::cookie_value(headers, session_cookie)?,
        identity::cookie_value(headers, cookie_name)?,
    ))
}

pub(super) fn map_tenant(
    response: platform_contracts::tenant_registry::TenantResponse,
) -> super::models::TenantView {
    super::models::TenantView {
        tenant_id: response.tenant_id,
        display_name: response.display_name,
        slug: response.slug,
        status: response.status,
        status_version: response.status_version,
        created_at: response.created_at,
        updated_at: response.updated_at,
    }
}

pub(super) fn tenant_grpc_error(
    status: tonic::Status,
    operation: &'static str,
    correlation: &str,
    duration_ms: u64,
) -> Response {
    let (http_status, code, category, retryable) = match status.code() {
        tonic::Code::InvalidArgument => (
            StatusCode::BAD_REQUEST,
            "tenant_input_invalid",
            "validation",
            false,
        ),
        tonic::Code::AlreadyExists => (StatusCode::CONFLICT, "tenant_conflict", "conflict", false),
        tonic::Code::Unauthenticated => (
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
        ),
        tonic::Code::PermissionDenied => (
            StatusCode::FORBIDDEN,
            "authorization_denied",
            "authorization",
            false,
        ),
        tonic::Code::NotFound => (
            StatusCode::NOT_FOUND,
            "tenant_not_found",
            "not_found",
            false,
        ),
        tonic::Code::OutOfRange => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            "realtime_cursor_out_of_range",
            "validation",
            false,
        ),
        tonic::Code::FailedPrecondition
            if status
                .message()
                .contains("realtime journal contains a cursor gap") =>
        {
            (
                StatusCode::CONFLICT,
                "realtime_cursor_gap",
                "conflict",
                false,
            )
        }
        tonic::Code::Aborted | tonic::Code::FailedPrecondition => (
            StatusCode::CONFLICT,
            "tenant_state_conflict",
            "conflict",
            false,
        ),
        tonic::Code::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant_dependency_unavailable",
            "dependency",
            true,
        ),
        tonic::Code::DeadlineExceeded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant_module_timeout",
            "timeout",
            true,
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "tenant_operation_failed",
            "internal",
            false,
        ),
    };
    tracing::warn!(
        event.name = "bff.dependency.call.failed", component = "platform_bff",
        operation, dependency = "tenant_registry", grpc_code = %status.code(),
        error_code = code, error_category = category, retryable,
        dependency_duration_ms = duration_ms, correlation_id = %correlation,
        trace_id = %payvia_observability::current_trace_id(),
        span_id = %payvia_observability::current_span_id()
    );
    identity::error(
        http_status,
        code,
        category,
        retryable,
        correlation.to_owned(),
    )
}

pub(super) fn auth_error(correlation: String) -> Response {
    identity::error(
        StatusCode::UNAUTHORIZED,
        "authentication_failed",
        "authentication",
        false,
        correlation,
    )
}

pub(super) fn csrf_error(correlation: String) -> Response {
    identity::error(
        StatusCode::FORBIDDEN,
        "csrf_validation_failed",
        "authorization",
        false,
        correlation,
    )
}
