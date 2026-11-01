use chrono::{DateTime, Utc};
use platform_contracts::catalog_pricing::PriceVersionResponse;
use tonic::Status;
use uuid::Uuid;

use crate::application::catalog_service::CatalogServiceError;

pub(super) fn parse_uuid(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("identifier is invalid"))
}
pub(super) fn parse_time(value: &str) -> Result<DateTime<Utc>, Status> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| Status::invalid_argument("timestamp is invalid"))
}
pub(super) fn price_response(price: crate::domain::catalog::PriceVersion) -> PriceVersionResponse {
    PriceVersionResponse {
        price_version_id: price.id.to_string(),
        plan_id: price.plan_id.to_string(),
        version: u32::try_from(price.version).unwrap_or_default(),
        currency: price.currency,
        amount_minor: price.amount_minor,
        period_unit: price.period_unit,
        interval_count: u32::try_from(price.interval_count).unwrap_or_default(),
        effective_from: price.effective_from.to_rfc3339(),
        effective_until: price
            .effective_until
            .map(|v| v.to_rfc3339())
            .unwrap_or_default(),
        published_at: price.published_at.to_rfc3339(),
    }
}
pub(super) fn map_error(error: CatalogServiceError) -> Status {
    match error {
        CatalogServiceError::Domain(crate::domain::catalog::CatalogError::InvalidPlan)
        | CatalogServiceError::Domain(crate::domain::catalog::CatalogError::InvalidPrice) => {
            Status::invalid_argument("catalog input is invalid")
        }
        CatalogServiceError::Domain(crate::domain::catalog::CatalogError::PlanNotFound)
        | CatalogServiceError::Domain(crate::domain::catalog::CatalogError::PriceVersionNotFound) => {
            Status::not_found("catalog resource was not found")
        }
        CatalogServiceError::Domain(
            crate::domain::catalog::CatalogError::EffectivePeriodConflict,
        ) => Status::already_exists("price effective period conflicts"),
        CatalogServiceError::Database(error) => {
            tracing::error!(event.name="catalog.persistence.failed", component="catalog_pricing", error_class=%error);
            Status::internal("catalog operation could not be completed")
        }
    }
}
