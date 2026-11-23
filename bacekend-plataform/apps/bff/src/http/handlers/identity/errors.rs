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
        tonic::Code::FailedPrecondition => (
            StatusCode::CONFLICT,
            "identity_precondition_failed",
            "precondition",
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

pub(crate) fn grpc_error_for(
    status: tonic::Status,
    operation: &'static str,
    dependency: &'static str,
    correlation: &str,
    duration_ms: u64,
) -> Response {
    let (http, default_code, default_category) = match status.code() {
        tonic::Code::InvalidArgument => (StatusCode::BAD_REQUEST, "request_invalid", "validation"),
        tonic::Code::AlreadyExists => (StatusCode::CONFLICT, "resource_conflict", "conflict"),
        tonic::Code::Unauthenticated => (
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
        ),
        tonic::Code::PermissionDenied => (
            StatusCode::FORBIDDEN,
            "authorization_denied",
            "authorization",
        ),
        tonic::Code::NotFound => (StatusCode::NOT_FOUND, "resource_not_found", "not_found"),
        tonic::Code::FailedPrecondition => (
            StatusCode::CONFLICT,
            "operation_precondition_failed",
            "precondition",
        ),
        tonic::Code::ResourceExhausted => (
            StatusCode::TOO_MANY_REQUESTS,
            "resource_limit_exceeded",
            "rate_limit",
        ),
        tonic::Code::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "dependency",
        ),
        tonic::Code::DeadlineExceeded => {
            (StatusCode::GATEWAY_TIMEOUT, "dependency_timeout", "timeout")
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "operation_failed",
            "internal",
        ),
    };
    let (code, category) =
        internal_error_details(&status).unwrap_or((default_code, default_category));
    let retryable = false;
    tracing::warn!(
        event.name = "bff.dependency.call.failed",
        component = "platform_bff",
        operation,
        dependency,
        grpc_code = ?status.code(),
        error_code = code,
        error_category = category,
        retryable,
        dependency_duration_ms = duration_ms,
        correlation_id = %correlation,
        trace_id = %current_trace_id(),
        span_id = %current_span_id()
    );
    error_for_operation(
        http,
        code,
        category,
        retryable,
        correlation.to_owned(),
        operation,
        "The requested operation could not be completed.",
    )
}

pub(crate) fn error(
    status: StatusCode,
    code: &'static str,
    category: &'static str,
    retryable: bool,
    correlation_id: String,
) -> Response {
    error_for_operation(
        status,
        code,
        category,
        retryable,
        correlation_id,
        "identity",
        "The identity operation could not be completed.",
    )
}

fn error_for_operation(
    status: StatusCode,
    code: &str,
    category: &str,
    retryable: bool,
    correlation_id: String,
    operation: &'static str,
    message: &'static str,
) -> Response {
    let trace_id = current_trace_id();
    tracing::warn!(event.name = "bff.request.failed", component = "platform_bff", operation, error_code = code, error_category = category, retryable, correlation_id = %correlation_id, trace_id = %trace_id, span_id = %current_span_id());
    let body = ErrorEnvelope {
        error: ErrorBody {
            code: code.to_owned(),
            category: category.to_owned(),
            message,
            correlation_id,
            trace_id,
            retryable,
        },
    };
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        "x-internal-error-code",
        HeaderValue::try_from(code).expect("stable error code is a valid HTTP header value"),
    );
    response.headers_mut().insert(
        "x-internal-error-category",
        HeaderValue::try_from(category)
            .expect("stable error category is a valid HTTP header value"),
    );
    response
}

fn internal_error_details(status: &tonic::Status) -> Option<(&'static str, &'static str)> {
    let has_code = status.metadata().contains_key("x-internal-error-code");
    let has_category = status.metadata().contains_key("x-internal-error-category");
    if !has_code && !has_category {
        return None;
    }
    let Some(code) = status
        .metadata()
        .get("x-internal-error-code")
        .and_then(|value| value.to_str().ok())
    else {
        tracing::warn!(
            event.name = "bff.error_metadata.rejected",
            component = "platform_bff",
            operation = "grpc_error_mapping",
            error_code = "internal_error_code_missing",
            trace_id = %current_trace_id(),
            span_id = %current_span_id()
        );
        return None;
    };
    let code = match code {
        "idempotency_result_unavailable" => "idempotency_result_unavailable",
        "postgres_unique_constraint_violation" => "postgres_unique_constraint_violation",
        "postgres_constraint_violation" => "postgres_constraint_violation",
        "postgres_serialization_failure" => "postgres_serialization_failure",
        "postgres_deadlock_detected" => "postgres_deadlock_detected",
        "postgres_query_failed" => "postgres_query_failed",
        "postgres_pool_timeout" => "postgres_pool_timeout",
        "postgres_pool_closed" => "postgres_pool_closed",
        "postgres_connection_failed" => "postgres_connection_failed",
        "postgres_persistence_failed" => "postgres_persistence_failed",
        _ => {
            tracing::warn!(
                event.name = "bff.error_metadata.rejected",
                component = "platform_bff",
                operation = "grpc_error_mapping",
                error_code = "internal_error_code_invalid",
                trace_id = %current_trace_id(),
                span_id = %current_span_id()
            );
            return None;
        }
    };
    let Some(category) = status
        .metadata()
        .get("x-internal-error-category")
        .and_then(|value| value.to_str().ok())
    else {
        tracing::warn!(
            event.name = "bff.error_metadata.rejected",
            component = "platform_bff",
            operation = "grpc_error_mapping",
            error_code = "internal_error_category_missing",
            trace_id = %current_trace_id(),
            span_id = %current_span_id()
        );
        return None;
    };
    let category = match category {
        "integrity" => "integrity",
        "persistence" => "persistence",
        _ => {
            tracing::warn!(
                event.name = "bff.error_metadata.rejected",
                component = "platform_bff",
                operation = "grpc_error_mapping",
                error_code = "internal_error_category_invalid",
                trace_id = %current_trace_id(),
                span_id = %current_span_id()
            );
            return None;
        }
    };
    Some((code, category))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_precondition_maps_to_conflict() {
        let response = grpc_error_for(
            tonic::Status::failed_precondition("inactive tenant"),
            "assign_tenant_plan",
            "entitlements_quotas",
            "corr_assignment",
            12,
        );
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error body is readable");
        let body: serde_json::Value =
            serde_json::from_slice(&body).expect("error body is valid JSON");
        assert_eq!(body["error"]["code"], "operation_precondition_failed");
        assert_eq!(body["error"]["category"], "precondition");
    }

    #[tokio::test]
    async fn maps_safe_internal_metadata_without_exposing_status_message() {
        let mut metadata = tonic::metadata::MetadataMap::new();
        metadata.insert(
            "x-internal-error-code",
            tonic::metadata::MetadataValue::from_static("postgres_constraint_violation"),
        );
        metadata.insert(
            "x-internal-error-category",
            tonic::metadata::MetadataValue::from_static("persistence"),
        );
        let status = tonic::Status::with_metadata(
            tonic::Code::Internal,
            "SQL statement and private database detail",
            metadata,
        );
        let response = grpc_error_for(
            status,
            "assign_tenant_plan",
            "entitlements_quotas",
            "corr_assignment",
            12,
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error body is readable");
        let body_text = String::from_utf8(body.to_vec()).expect("error body is UTF-8");
        assert!(body_text.contains("postgres_constraint_violation"));
        assert!(body_text.contains("persistence"));
        assert!(!body_text.contains("private database detail"));
        assert!(!body_text.contains("SQL statement"));
    }
}
