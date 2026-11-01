use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::entitlement::Entitlement;

pub(super) struct EntitlementSnapshot {
    tenant_id: Uuid,
    price_version_id: Uuid,
    capability_key: String,
    enabled: bool,
    limit_value: Option<i64>,
    overage_policy: String,
    allowed_environment: String,
}

impl EntitlementSnapshot {
        fn build(
            self,
            environment: &str,
            consumed: i64,
            period_start: DateTime<Utc>,
            period_end: DateTime<Utc>,
        ) -> Entitlement {
            let environment_allowed =
                self.allowed_environment == "all" || self.allowed_environment == environment;
            let allowed = self.enabled
                && environment_allowed
                && (self.limit_value.is_none()
                    || self.overage_policy == "allow"
                    || consumed < self.limit_value.unwrap_or_default());
            let reason = if !self.enabled {
                "capability_disabled"
            } else if !environment_allowed {
                "environment_restricted"
            } else if !allowed {
                "quota_exhausted"
            } else {
                "allowed"
            };
            Entitlement {
                tenant_id: self.tenant_id,
                price_version_id: self.price_version_id,
                capability_key: self.capability_key,
                enabled: self.enabled,
                limit_value: self.limit_value,
                consumed,
                remaining: self
                    .limit_value
                    .map(|value| value.saturating_sub(consumed).max(0)),
                allowed,
                reason: reason.to_owned(),
                environment: environment.to_owned(),
                period_start,
                period_end,
            }
        }
    }
}
