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
    #[error("tenant legal identification is invalid")]
    InvalidLegalIdentification,
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
    pub fn validate_registration(
        display_name: &str,
        slug: &str,
        identification_type: &str,
        identification_number: &str,
    ) -> Result<String, TenantError> {
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
        if !matches!(identification_type, "nit" | "cc") {
            return Err(TenantError::InvalidLegalIdentification);
        }
        Self::normalize_identification_number(identification_number)
    }

    pub fn normalize_identification_number(value: &str) -> Result<String, TenantError> {
        let normalized_input = value.trim();
        let mut digits = String::with_capacity(normalized_input.len());
        let mut previous_was_digit = false;
        for character in normalized_input.chars() {
            if character.is_ascii_digit() {
                digits.push(character);
                previous_was_digit = true;
            } else if matches!(character, ' ' | '.' | '-') && previous_was_digit {
                previous_was_digit = false;
            } else {
                return Err(TenantError::InvalidLegalIdentification);
            }
        }
        if !previous_was_digit || !(5..=20).contains(&digits.len()) {
            return Err(TenantError::InvalidLegalIdentification);
        }
        Ok(digits)
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
            Tenant::validate_registration(" ", "valid-slug", "nit", "9001234567"),
            Err(TenantError::InvalidDisplayName)
        );
        assert_eq!(
            Tenant::validate_registration("Valid tenant", "Invalid-Slug", "nit", "9001234567"),
            Err(TenantError::InvalidSlug)
        );
        assert_eq!(
            Tenant::validate_registration("Valid tenant", "valid-slug", "nit", "900.123.456-7"),
            Ok("9001234567".to_owned())
        );
    }

    #[test]
    fn registration_rejects_invalid_legal_identification() {
        assert_eq!(
            Tenant::validate_registration("Valid tenant", "valid-slug", "passport", "123456"),
            Err(TenantError::InvalidLegalIdentification)
        );
        assert_eq!(
            Tenant::validate_registration("Valid tenant", "valid-slug", "cc", "12AB34"),
            Err(TenantError::InvalidLegalIdentification)
        );
    }

    #[test]
    fn registration_enforces_display_name_and_slug_boundaries() {
        assert_eq!(
            Tenant::validate_registration("A", "valid-slug", "nit", "12345"),
            Err(TenantError::InvalidDisplayName)
        );
        assert_eq!(
            Tenant::validate_registration(&"a".repeat(121), "valid-slug", "nit", "12345"),
            Err(TenantError::InvalidDisplayName)
        );
        assert!(Tenant::validate_registration("Valid tenant", "abc", "nit", "12345").is_ok());
        assert!(
            Tenant::validate_registration("Valid tenant", &"a".repeat(64), "nit", "12345").is_ok()
        );
        let too_long_slug = "a".repeat(65);
        for invalid_slug in [
            "ab",
            too_long_slug.as_str(),
            "-abc",
            "abc-",
            "a--b",
            "Uppercase",
            "under_score",
            "ábc",
        ] {
            assert_eq!(
                Tenant::validate_registration("Valid tenant", invalid_slug, "nit", "12345"),
                Err(TenantError::InvalidSlug),
                "must reject slug {invalid_slug:?}"
            );
        }
    }

    #[test]
    fn legal_identification_rejects_negative_and_malformed_separator_inputs() {
        assert_eq!(
            Tenant::normalize_identification_number("900.123.456-7"),
            Ok("9001234567".to_owned())
        );
        assert_eq!(
            Tenant::normalize_identification_number("12345"),
            Ok("12345".to_owned())
        );
        assert_eq!(
            Tenant::normalize_identification_number(&"1".repeat(20)),
            Ok("1".repeat(20))
        );
        let too_long_identification = "1".repeat(21);
        for invalid in [
            "-12345",
            "12345-",
            "12--345",
            "12..345",
            "1234",
            too_long_identification.as_str(),
            "12A345",
        ] {
            assert_eq!(
                Tenant::normalize_identification_number(invalid),
                Err(TenantError::InvalidLegalIdentification),
                "must reject legal identification {invalid:?}"
            );
        }
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

    #[test]
    fn tenant_status_transition_matrix_rejects_every_unlisted_edge() {
        use TenantStatus::*;
        let statuses = [PendingVerification, Active, Suspended, Closing, Closed];
        let allowed = [
            (PendingVerification, Active),
            (PendingVerification, Suspended),
            (PendingVerification, Closed),
            (Active, Suspended),
            (Active, Closing),
            (Suspended, Active),
            (Suspended, Closing),
            (Closing, Closed),
        ];
        for current in statuses {
            for next in statuses {
                assert_eq!(
                    current.permits(next),
                    allowed.contains(&(current, next)),
                    "unexpected transition {} -> {}",
                    current.as_str(),
                    next.as_str()
                );
            }
        }
    }

    #[test]
    fn tenant_transition_reason_has_inclusive_three_to_500_character_bounds() {
        use TenantStatus::*;
        assert_eq!(
            Tenant::validate_transition(Active, Closing, "abc", false),
            Ok(())
        );
        assert_eq!(
            Tenant::validate_transition(Active, Closing, &"r".repeat(500), false),
            Ok(())
        );
        let too_long_reason = "r".repeat(501);
        for reason in ["", "  ", "ab", too_long_reason.as_str()] {
            assert_eq!(
                Tenant::validate_transition(Active, Closing, reason, false),
                Err(TenantError::MissingReason),
                "must reject reason {reason:?}"
            );
        }
    }

    #[test]
    fn operator_is_required_for_sensitive_lifecycle_transitions() {
        use TenantStatus::*;
        for (current, next) in [
            (PendingVerification, Active),
            (Active, Suspended),
            (Suspended, Active),
            (PendingVerification, Closed),
            (Closing, Closed),
        ] {
            assert_eq!(
                Tenant::validate_transition(current, next, "reviewed by operator", false),
                Err(TenantError::OperatorRequired),
                "operator must authorize {} -> {}",
                current.as_str(),
                next.as_str()
            );
            assert_eq!(
                Tenant::validate_transition(current, next, "reviewed by operator", true),
                Ok(()),
                "operator may authorize {} -> {}",
                current.as_str(),
                next.as_str()
            );
        }

        for (current, next) in [(Active, Closed), (Suspended, Closed)] {
            for actor_is_operator in [false, true] {
                assert_eq!(
                    Tenant::validate_transition(
                        current,
                        next,
                        "reviewed by operator",
                        actor_is_operator,
                    ),
                    Err(TenantError::InvalidTransition),
                    "operator authorization must not bypass invalid transition {} -> {}",
                    current.as_str(),
                    next.as_str()
                );
            }
        }
    }

    #[test]
    fn tenant_status_parser_accepts_known_values_and_rejects_unknown_values() {
        use TenantStatus::*;
        for (raw, expected) in [
            ("pending_verification", PendingVerification),
            ("active", Active),
            ("suspended", Suspended),
            ("closing", Closing),
            ("closed", Closed),
        ] {
            assert_eq!(TenantStatus::parse(raw), Ok(expected));
            assert_eq!(expected.as_str(), raw);
        }
        for invalid in ["", "pending", "ACTIVE", "deleted", "unknown"] {
            assert_eq!(
                TenantStatus::parse(invalid),
                Err(TenantError::InvalidStatus),
                "must reject status {invalid:?}"
            );
        }
    }
}
