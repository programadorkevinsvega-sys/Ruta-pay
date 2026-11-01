use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::EntitlementServiceError;
use crate::domain::entitlement::Entitlement;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EntitlementResultSnapshot {
    tenant_id: Uuid,
    price_version_id: Uuid,
    capability_key: String,
    enabled: bool,
    limit_value: Option<i64>,
    consumed: i64,
    remaining: Option<i64>,
    allowed: bool,
    reason: String,
    environment: String,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
}

impl EntitlementResultSnapshot {
    pub(super) fn encode(result: &Entitlement) -> Result<Value, EntitlementServiceError> {
        serde_json::to_value(Self::from(result))
            .map_err(|_| EntitlementServiceError::IdempotencyResultUnavailable)
    }

    pub(super) fn decode(
        value: Value,
        tenant: Uuid,
        capability_key: &str,
        environment: &str,
    ) -> Result<Entitlement, EntitlementServiceError> {
        let snapshot: Self = serde_json::from_value(value)
            .map_err(|_| EntitlementServiceError::IdempotencyResultUnavailable)?;
        let expected_remaining = snapshot
            .limit_value
            .map(|limit| limit.saturating_sub(snapshot.consumed).max(0));
        if snapshot.tenant_id != tenant
            || snapshot.capability_key != capability_key
            || snapshot.environment != environment
            || snapshot.consumed < 0
            || snapshot.limit_value.is_some_and(|limit| limit < 0)
            || snapshot.period_end <= snapshot.period_start
            || snapshot.remaining != expected_remaining
            || !matches!(
                snapshot.reason.as_str(),
                "allowed" | "capability_disabled" | "environment_restricted" | "quota_exhausted"
            )
            || snapshot.allowed != (snapshot.reason == "allowed")
        {
            return Err(EntitlementServiceError::IdempotencyResultUnavailable);
        }
        Ok(snapshot.into())
    }
}

impl From<&Entitlement> for EntitlementResultSnapshot {
    fn from(result: &Entitlement) -> Self {
        Self {
            tenant_id: result.tenant_id,
            price_version_id: result.price_version_id,
            capability_key: result.capability_key.clone(),
            enabled: result.enabled,
            limit_value: result.limit_value,
            consumed: result.consumed,
            remaining: result.remaining,
            allowed: result.allowed,
            reason: result.reason.clone(),
            environment: result.environment.clone(),
            period_start: result.period_start.clone(),
            period_end: result.period_end.clone(),
        }
    }
}

impl From<EntitlementResultSnapshot> for Entitlement {
    fn from(snapshot: EntitlementResultSnapshot) -> Self {
        Self {
            tenant_id: snapshot.tenant_id,
            price_version_id: snapshot.price_version_id,
            capability_key: snapshot.capability_key,
            enabled: snapshot.enabled,
            limit_value: snapshot.limit_value,
            consumed: snapshot.consumed,
            remaining: snapshot.remaining,
            allowed: snapshot.allowed,
            reason: snapshot.reason,
            environment: snapshot.environment,
            period_start: snapshot.period_start,
            period_end: snapshot.period_end,
        }
    }
}
