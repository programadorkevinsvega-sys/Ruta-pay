use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub key: String,
    pub enabled: bool,
    pub limit_value: Option<i64>,
    pub unit: String,
    pub overage_policy: String,
    pub environment: String,
    pub quota_period: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entitlement {
    pub tenant_id: Uuid,
    pub price_version_id: Uuid,
    pub capability_key: String,
    pub enabled: bool,
    pub limit_value: Option<i64>,
    pub consumed: i64,
    pub remaining: Option<i64>,
    pub allowed: bool,
    pub reason: String,
    pub environment: String,
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub period_end: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EntitlementError {
    #[error("capability is invalid")]
    InvalidCapability,
    #[error("quota quantity is invalid")]
    InvalidQuantity,
    #[error("idempotency key is invalid")]
    InvalidIdempotencyKey,
    #[error("tenant has no effective plan")]
    PlanNotAssigned,
    #[error("capability is not included in the effective plan")]
    CapabilityUnavailable,
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("idempotency key was reused with different operation data")]
    IdempotencyConflict,
    #[error("plan assignment overlaps an existing assignment")]
    AssignmentConflict,
}

impl Capability {
    pub fn validate(&self) -> Result<(), EntitlementError> {
        if self.key.is_empty()
            || self.key.len() > 96
            || !self.key.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
            })
            || self.limit_value.is_some_and(|v| v < 0)
            || self.unit.is_empty()
            || self.unit.len() > 32
            || !matches!(self.overage_policy.as_str(), "deny" | "allow")
            || !matches!(self.environment.as_str(), "all" | "test" | "live")
            || !matches!(
                self.quota_period.as_str(),
                "day" | "week" | "month" | "year"
            )
        {
            return Err(EntitlementError::InvalidCapability);
        }
        Ok(())
    }
}

pub fn validate_consumption(quantity: i64, key: &str) -> Result<(), EntitlementError> {
    if quantity <= 0 {
        return Err(EntitlementError::InvalidQuantity);
    }
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
    {
        return Err(EntitlementError::InvalidIdempotencyKey);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Capability, EntitlementError, validate_consumption};

    fn capability() -> Capability {
        Capability {
            key: "payments.monthly".into(),
            enabled: true,
            limit_value: Some(100),
            unit: "transactions".into(),
            overage_policy: "deny".into(),
            environment: "live".into(),
            quota_period: "month".into(),
        }
    }

    #[test]
    fn capability_policy_and_limits_are_validated() {
        assert_eq!(capability().validate(), Ok(()));
        let mut invalid = capability();
        invalid.limit_value = Some(-1);
        assert_eq!(invalid.validate(), Err(EntitlementError::InvalidCapability));
        invalid = capability();
        invalid.environment = "production".into();
        assert_eq!(invalid.validate(), Err(EntitlementError::InvalidCapability));
        invalid = capability();
        invalid.quota_period = "forever".into();
        assert_eq!(invalid.validate(), Err(EntitlementError::InvalidCapability));
    }

    #[test]
    fn consumption_requires_positive_quantity_and_bounded_idempotency_key() {
        assert_eq!(validate_consumption(1, "request-1"), Ok(()));
        assert_eq!(
            validate_consumption(0, "request-1"),
            Err(EntitlementError::InvalidQuantity)
        );
        assert_eq!(
            validate_consumption(1, ""),
            Err(EntitlementError::InvalidIdempotencyKey)
        );
    }
}
