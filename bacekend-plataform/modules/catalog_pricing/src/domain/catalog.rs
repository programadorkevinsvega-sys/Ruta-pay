use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceVersion {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub version: i32,
    pub currency: String,
    pub amount_minor: i64,
    pub period_unit: String,
    pub interval_count: i32,
    pub effective_from: DateTime<Utc>,
    pub effective_until: Option<DateTime<Utc>>,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("plan input is invalid")]
    InvalidPlan,
    #[error("price version input is invalid")]
    InvalidPrice,
    #[error("price effective period overlaps an existing version")]
    EffectivePeriodConflict,
    #[error("plan was not found")]
    PlanNotFound,
    #[error("price version was not found")]
    PriceVersionNotFound,
}

impl Plan {
    pub fn validate(slug: &str, name: &str, description: &str) -> Result<(), CatalogError> {
        if !(3..=64).contains(&slug.len())
            || !slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || slug.starts_with('-')
            || slug.ends_with('-')
            || slug.contains("--")
            || !(2..=100).contains(&name.trim().chars().count())
            || description.chars().count() > 1000
        {
            return Err(CatalogError::InvalidPlan);
        }
        Ok(())
    }
}

impl PriceVersion {
    pub fn validate(
        currency: &str,
        amount_minor: i64,
        period_unit: &str,
        interval_count: i32,
        effective_from: DateTime<Utc>,
        effective_until: Option<DateTime<Utc>>,
    ) -> Result<(), CatalogError> {
        if currency.len() != 3
            || !currency.bytes().all(|b| b.is_ascii_uppercase())
            || amount_minor < 0
            || !matches!(period_unit, "day" | "week" | "month" | "year")
            || !(1..=365).contains(&interval_count)
            || effective_until.is_some_and(|until| until <= effective_from)
        {
            return Err(CatalogError::InvalidPrice);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CatalogError, Plan, PriceVersion};
    use chrono::{Duration, Utc};

    #[test]
    fn plan_slug_and_name_are_strictly_validated() {
        assert_eq!(Plan::validate("starter-plan", "Starter Plan", ""), Ok(()));
        for slug in ["AB", "ab", "-abc", "abc-", "a--b", "a_b"] {
            assert_eq!(
                Plan::validate(slug, "Valid Plan", ""),
                Err(CatalogError::InvalidPlan)
            );
        }
        assert_eq!(
            Plan::validate("valid", "x", ""),
            Err(CatalogError::InvalidPlan)
        );
    }

    #[test]
    fn price_versions_validate_currency_amount_period_and_dates() {
        let now = Utc::now();
        assert_eq!(
            PriceVersion::validate("USD", 100, "month", 1, now, Some(now + Duration::days(30))),
            Ok(())
        );
        assert_eq!(
            PriceVersion::validate("usd", 100, "month", 1, now, None),
            Err(CatalogError::InvalidPrice)
        );
        assert_eq!(
            PriceVersion::validate("USD", -1, "month", 1, now, None),
            Err(CatalogError::InvalidPrice)
        );
        assert_eq!(
            PriceVersion::validate("USD", 100, "minute", 1, now, None),
            Err(CatalogError::InvalidPrice)
        );
        assert_eq!(
            PriceVersion::validate("USD", 100, "month", 0, now, None),
            Err(CatalogError::InvalidPrice)
        );
        assert_eq!(
            PriceVersion::validate("USD", 100, "month", 1, now, Some(now)),
            Err(CatalogError::InvalidPrice)
        );
    }
}
