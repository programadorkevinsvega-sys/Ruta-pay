use chrono::{DateTime, Utc};
use platform_contracts::entitlements_quotas::{CapabilityInput, TenantEntitlementResponse};
use tonic::{
    Code, Status,
    metadata::{MetadataMap, MetadataValue},
};
use uuid::Uuid;

use crate::{
    application::entitlement_service::EntitlementServiceError,
    domain::entitlement::{Capability, Entitlement},
};

pub(super) fn map_capability(value: CapabilityInput) -> Result<Capability, Status> {
    let capability = Capability {
        key: value.capability_key,
        enabled: value.enabled,
        limit_value: value.limit_value,
        unit: value.unit,
        overage_policy: value.overage_policy,
        environment: value.environment,
        quota_period: value.quota_period,
        billable: value.billable,
    };
    capability
        .validate()
        .map_err(|_| Status::invalid_argument("capability input is invalid"))?;
    Ok(capability)
}

pub(super) fn entitlement_response(value: Entitlement) -> TenantEntitlementResponse {
    TenantEntitlementResponse {
        tenant_id: value.tenant_id.to_string(),
        price_version_id: value.price_version_id.to_string(),
        capability_key: value.capability_key,
        enabled: value.enabled,
        limit_value: value.limit_value,
        consumed: value.consumed,
        remaining: value.remaining,
        allowed: value.allowed,
        reason: value.reason,
        environment: value.environment,
    }
}

pub(super) fn parse_uuid(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("identifier is invalid"))
}

pub(super) fn parse_time(value: &str) -> Result<DateTime<Utc>, Status> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| Status::invalid_argument("timestamp is invalid"))
}

pub(super) fn map_error(
    error: EntitlementServiceError,
    operation: &'static str,
    correlation_id: &str,
    dependency_duration_ms: u64,
) -> Status {
    match error {
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::InvalidCapability,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::InvalidQuantity,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::InvalidIdempotencyKey,
        ) => Status::invalid_argument("entitlement input is invalid"),
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::CapabilityUnavailable,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::PlanNotAssigned,
        ) => Status::not_found("entitlement was not found"),
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::QuotaExceeded,
        ) => Status::resource_exhausted("quota is exhausted"),
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::IdempotencyConflict,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::AssignmentConflict,
        ) => Status::already_exists("entitlement operation conflicts"),
        EntitlementServiceError::IdempotencyResultUnavailable => {
            tracing::error!(
                event.name = "entitlements.idempotency_snapshot.unavailable",
                component = "entitlements_quotas",
                operation,
                error_code = "idempotency_result_unavailable",
                error_category = "integrity",
                dependency = "postgresql",
                dependency_duration_ms,
                correlation_id,
                trace_id = %payvia_observability::current_trace_id(),
                span_id = %payvia_observability::current_span_id(),
                retryable = false
            );
            diagnosed_status(
                Code::FailedPrecondition,
                "idempotent result is unavailable",
                "idempotency_result_unavailable",
                "integrity",
            )
        }
        EntitlementServiceError::Database(error) => {
            let database = error.as_database_error();
            let sqlstate = database
                .and_then(|database| database.code())
                .map(|code| code.into_owned());
            let error_code = match sqlstate.as_deref() {
                Some("23505") => "postgres_unique_constraint_violation",
                Some(code) if code.starts_with("23") => "postgres_constraint_violation",
                Some("40001") => "postgres_serialization_failure",
                Some("40P01") => "postgres_deadlock_detected",
                Some(_) => "postgres_query_failed",
                None => match &error {
                    sqlx::Error::PoolTimedOut => "postgres_pool_timeout",
                    sqlx::Error::PoolClosed => "postgres_pool_closed",
                    sqlx::Error::Io(_) => "postgres_connection_failed",
                    _ => "postgres_persistence_failed",
                },
            };
            let error_class = match &error {
                sqlx::Error::Database(_) => "database",
                sqlx::Error::PoolTimedOut => "pool_timeout",
                sqlx::Error::PoolClosed => "pool_closed",
                sqlx::Error::Io(_) => "connection",
                _ => "persistence",
            };
            tracing::error!(
                event.name = "grpc.dependency.call.failed",
                component = "entitlements_quotas",
                operation,
                dependency = "postgresql",
                error_code,
                error_category = "persistence",
                error_class,
                sqlstate = sqlstate.as_deref().unwrap_or("unknown"),
                constraint = database.and_then(|database| database.constraint()).unwrap_or("unknown"),
                table = database.and_then(|database| database.table()).unwrap_or("unknown"),
                column = database
                    .and_then(|database| {
                        database
                            .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                            .and_then(|database| database.column())
                    })
                    .unwrap_or("unknown"),
                dependency_duration_ms,
                correlation_id,
                trace_id = %payvia_observability::current_trace_id(),
                span_id = %payvia_observability::current_span_id(),
                retryable = false
            );
            diagnosed_status(
                Code::Internal,
                "entitlement operation could not be completed",
                error_code,
                "persistence",
            )
        }
    }
}

fn diagnosed_status(
    code: Code,
    message: &'static str,
    error_code: &'static str,
    error_category: &'static str,
) -> Status {
    let mut metadata = MetadataMap::new();
    metadata.insert(
        "x-internal-error-code",
        MetadataValue::from_static(error_code),
    );
    metadata.insert(
        "x-internal-error-category",
        MetadataValue::from_static(error_category),
    );
    Status::with_metadata(code, message, metadata)
}

pub(super) fn error_details(status: &Status) -> (String, String) {
    let metadata_code = status
        .metadata()
        .get("x-internal-error-code")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        });
    let metadata_category = status
        .metadata()
        .get("x-internal-error-category")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            matches!(
                *value,
                "validation"
                    | "conflict"
                    | "authentication"
                    | "authorization"
                    | "not_found"
                    | "precondition"
                    | "rate_limit"
                    | "dependency"
                    | "timeout"
                    | "internal"
                    | "persistence"
                    | "integrity"
            )
        });
    if let (Some(code), Some(category)) = (metadata_code, metadata_category) {
        return (code.to_owned(), category.to_owned());
    }
    if status.metadata().contains_key("x-internal-error-code")
        || status.metadata().contains_key("x-internal-error-category")
    {
        tracing::warn!(
            event.name = "grpc.error_metadata.rejected",
            component = "entitlements_quotas",
            operation = "error_mapping",
            error_code = "internal_error_metadata_invalid",
            trace_id = %payvia_observability::current_trace_id(),
            span_id = %payvia_observability::current_span_id()
        );
    }
    let (code, category) = error_classification(status.code());
    (code.to_owned(), category.to_owned())
}

pub(super) fn error_classification(code: Code) -> (&'static str, &'static str) {
    match code {
        Code::InvalidArgument => ("entitlement_input_invalid", "validation"),
        Code::AlreadyExists => ("entitlement_conflict", "conflict"),
        Code::Unauthenticated => ("authentication_failed", "authentication"),
        Code::PermissionDenied => ("authorization_denied", "authorization"),
        Code::NotFound => ("resource_not_found", "not_found"),
        Code::FailedPrecondition => ("operation_precondition_failed", "precondition"),
        Code::ResourceExhausted => ("quota_limit_exceeded", "rate_limit"),
        Code::Unavailable => ("dependency_unavailable", "dependency"),
        Code::DeadlineExceeded => ("dependency_timeout", "timeout"),
        _ => ("entitlements_operation_failed", "internal"),
    }
}
