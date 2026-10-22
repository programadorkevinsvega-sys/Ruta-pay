use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tenant {
    pub id: Uuid,
    pub display_name: String,
    pub slug: String,
    pub status: TenantStatus,
    pub created_by: Uuid,
    pub status_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TenantStatus {
    PendingVerification,
    Active,
    Suspended,
    Closing,
    Closed,
}

impl TenantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingVerification => "pending_verification",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Closing => "closing",
            Self::Closed => "closed",
        }
    }

    pub fn parse(value: &str) -> Result<Self, TenantError> {
        match value {
            "pending_verification" => Ok(Self::PendingVerification),
            "active" => Ok(Self::Active),
            "suspended" => Ok(Self::Suspended),
            "closing" => Ok(Self::Closing),
            "closed" => Ok(Self::Closed),
            _ => Err(TenantError::InvalidStatus),
        }
    }

    pub fn permits(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::PendingVerification, Self::Active)
                | (Self::PendingVerification, Self::Suspended)
                | (Self::PendingVerification, Self::Closed)
                | (Self::Active, Self::Suspended)
                | (Self::Active, Self::Closing)
                | (Self::Suspended, Self::Active)
                | (Self::Suspended, Self::Closing)
                | (Self::Closing, Self::Closed)
        )
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TenantError {
    #[error("tenant display name is invalid")]
    InvalidDisplayName,
    #[error("tenant slug is invalid")]
    InvalidSlug,
    #[error("tenant status is invalid")]
    InvalidStatus,
    #[error("tenant status transition is not allowed")]
    InvalidTransition,
    #[error("a reason is required for this status change")]
    MissingReason,
    #[error("tenant operation requires an operator")]
    OperatorRequired,
}

impl Tenant {
    pub fn validate_registration(display_name: &str, slug: &str) -> Result<(), TenantError> {
        let name = display_name.trim();
        if !(2..=120).contains(&name.chars().count()) {
            return Err(TenantError::InvalidDisplayName);
        }
        if !(3..=64).contains(&slug.len())
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || slug.starts_with('-')
            || slug.ends_with('-')
            || slug.contains("--")
        {
            return Err(TenantError::InvalidSlug);
        }
        Ok(())
    }

    pub fn validate_transition(
        current: TenantStatus,
        next: TenantStatus,
        reason: &str,
        actor_is_operator: bool,
    ) -> Result<(), TenantError> {
        if current == next || !current.permits(next) {
            return Err(TenantError::InvalidTransition);
        }
        if reason.trim().chars().count() < 3 || reason.chars().count() > 500 {
            return Err(TenantError::MissingReason);
        }
        if matches!(
            (current, next),
            (TenantStatus::PendingVerification, TenantStatus::Active)
                | (TenantStatus::Active, TenantStatus::Suspended)
                | (TenantStatus::Suspended, TenantStatus::Active)
                | (_, TenantStatus::Closed)
        ) && !actor_is_operator
        {
            return Err(TenantError::OperatorRequired);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Tenant, TenantError, TenantStatus};

    #[test]
    fn registration_rejects_invalid_names_and_slugs() {
        assert_eq!(
            Tenant::validate_registration(" ", "valid-slug"),
            Err(TenantError::InvalidDisplayName)
        );
        assert_eq!(
            Tenant::validate_registration("Valid tenant", "Invalid-Slug"),
            Err(TenantError::InvalidSlug)
        );
        assert_eq!(
            Tenant::validate_registration("Valid tenant", "valid-slug"),
            Ok(())
        );
    }

    #[test]
    fn tenant_lifecycle_requires_authorized_operator_transitions() {
        assert_eq!(
            Tenant::validate_transition(
                TenantStatus::PendingVerification,
                TenantStatus::Active,
                "documents reviewed",
                false,
            ),
            Err(TenantError::OperatorRequired)
        );
        assert_eq!(
            Tenant::validate_transition(
                TenantStatus::PendingVerification,
                TenantStatus::Active,
                "documents reviewed",
                true,
            ),
            Ok(())
        );
        assert_eq!(
            Tenant::validate_transition(TenantStatus::Closed, TenantStatus::Active, "reopen", true,),
            Err(TenantError::InvalidTransition)
        );
    }
}
