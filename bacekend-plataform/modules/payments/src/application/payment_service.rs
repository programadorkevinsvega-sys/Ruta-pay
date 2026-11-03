mod create;
mod get;
mod snapshot;

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::payment_intent::{PaymentIntent, PaymentIntentError};

#[derive(Clone)]
pub struct PaymentService {
    pool: PgPool,
}

#[derive(Clone, Debug)]
pub struct CreatePaymentIntentCommand {
    pub tenant_id: Uuid,
    pub actor_key_id: Uuid,
    pub environment: String,
    pub amount_minor: i64,
    pub currency: String,
    pub payment_method_type: String,
    pub merchant_reference: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PaymentServiceError {
    #[error(transparent)]
    Domain(#[from] PaymentIntentError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    #[error("the persisted idempotency response is unavailable or corrupt")]
    SnapshotUnavailable,
}

impl PaymentService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_payment_intent(
        &self,
        command: CreatePaymentIntentCommand,
    ) -> Result<(PaymentIntent, bool), PaymentServiceError> {
        create::create(self, command).await
    }

    pub async fn get_payment_intent(
        &self,
        tenant_id: Uuid,
        payment_intent_id: Uuid,
    ) -> Result<PaymentIntent, PaymentServiceError> {
        get::get(self, tenant_id, payment_intent_id).await
    }
}
