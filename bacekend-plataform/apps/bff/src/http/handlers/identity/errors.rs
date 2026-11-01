use super::*;

pub(crate) fn grpc_error(
    status: tonic::Status,
    operation: &'static str,
    correlation: &str,
    duration_ms: u64,
) -> Response {
    let (http, code, category, retryable) = match status.code() {
        tonic::Code::InvalidArgument => (
            StatusCode::BAD_REQUEST,
            "identity_input_invalid",
            "validation",
            false,
        ),
        tonic::Code::AlreadyExists => {
            (StatusCode::CONFLICT, "identity_conflict", "conflict", false)
        }
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
            "resource_not_found",
            "not_found",
            false,
        ),
        tonic::Code::ResourceExhausted => (
            StatusCode::TOO_MANY_REQUESTS,
            "identity_rate_limited",
            "rate_limit",
            false,
        ),
        tonic::Code::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "identity_dependency_unavailable",
            "dependency",
            true,
        ),
        tonic::Code::DeadlineExceeded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "module_timeout",
            "timeout",
            true,
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "identity_operation_failed",
            "internal",
            false,
        ),
    };
    tracing::warn!(event.name = "bff.dependency.call.failed", component = "platform_bff", operation, dependency = "identity_access", error_code = %status.code(), error_category = category, retryable, dependency_duration_ms = duration_ms, correlation_id = %correlation, trace_id = %current_trace_id(), span_id = %current_span_id());
    error(http, code, category, retryable, correlation.to_owned())
}

pub(crate) fn error(
    status: StatusCode,
    code: &'static str,
    category: &'static str,
    retryable: bool,
    correlation_id: String,
) -> Response {
    let trace_id = current_trace_id();
    tracing::warn!(event.name = "bff.request.failed", component = "platform_bff", operation = "identity", error_code = code, error_category = category, retryable, correlation_id = %correlation_id, trace_id = %trace_id, span_id = %current_span_id());
    let body = ErrorEnvelope {
        error: ErrorBody {
            code,
            category,
            message: "The identity operation could not be completed.",
            correlation_id,
            trace_id,
            retryable,
        },
    };
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert("x-internal-error-code", HeaderValue::from_static(code));
    response.headers_mut().insert(
        "x-internal-error-category",
        HeaderValue::from_static(category),
    );
    response
}
