use super::*;

pub(super) fn grpc_span<T>(
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

pub(super) fn parse_uuid(value: &str, field: &'static str) -> Result<uuid::Uuid, Status> {
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

pub(super) fn log_and_map(
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
