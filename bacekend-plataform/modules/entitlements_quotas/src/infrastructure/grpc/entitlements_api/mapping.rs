use chrono::{DateTime, Utc};
use platform_contracts::entitlements_quotas::{CapabilityInput, TenantEntitlementResponse};
use tonic::Status;
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

pub(super) fn map_error(error: EntitlementServiceError) -> Status {
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
        EntitlementServiceError::Database(error) => {
            tracing::error!(event.name="entitlements.persistence.failed", component="entitlements_quotas", error_class=%error);
            Status::internal("entitlement operation could not be completed")
        }
    }
}
