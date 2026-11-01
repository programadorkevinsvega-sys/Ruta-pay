mod plan;
mod price;
mod support;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::catalog::{CatalogError, Plan, PriceVersion};

#[derive(Clone)]
pub struct CatalogService {
    pool: PgPool,
}

pub struct PublishPriceVersionCommand<'a> {
    pub actor_id: Uuid,
    pub plan_id: Uuid,
    pub currency: &'a str,
    pub amount_minor: i64,
    pub period_unit: &'a str,
    pub interval_count: i32,
    pub effective_from: DateTime<Utc>,
    pub effective_until: Option<DateTime<Utc>>,
    pub correlation_id: &'a str,
}

impl CatalogService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogServiceError {
    #[error(transparent)]
    Domain(#[from] CatalogError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
