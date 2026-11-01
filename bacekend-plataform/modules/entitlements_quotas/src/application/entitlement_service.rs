mod assignment;
mod capabilities;
mod consumption;
mod evaluation;
mod result_snapshot;
mod snapshot;
mod support;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entitlement::EntitlementError;

#[derive(Clone)]
pub struct EntitlementService {
    pool: PgPool,
}

pub struct ConsumeQuotaCommand<'a> {
    pub actor: Uuid,
    pub tenant: Uuid,
    pub key: &'a str,
    pub environment: &'a str,
    pub quantity: i64,
    pub idempotency_key: &'a str,
    pub now: DateTime<Utc>,
    pub correlation_id: &'a str,
}

impl EntitlementService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EntitlementServiceError {
    #[error(transparent)]
    Domain(#[from] EntitlementError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
