mod attempts;
mod create;
mod get;
mod lifecycle_status;
mod snapshot;
mod transactions;

use sqlx::PgPool;
use uuid::Uuid;

use chrono::{DateTime, Utc};
use crate::domain::{payment_intent::{PaymentIntent, PaymentIntentError}, payment_lifecycle::{LifecycleModelError, TransitionGuardError}};

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
    #[error(transparent)]
    Lifecycle(#[from] LifecycleModelError),
    #[error(transparent)]
    Transition(#[from] TransitionGuardError),
    #[error("a payment attempt already exists; commercial retries are not enabled")]
    AttemptAlreadyExists,
    #[error("the requested payment intent status update is not supported by this API")]
    StatusUpdateNotAllowed,
    #[error("the payment aggregate version changed; reload the resource")]
    VersionConflict,
    #[error("page size must be between 1 and 100")]
    InvalidPageSize,
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
        environment: &str,
        payment_intent_id: Uuid,
        actor_key_id: Uuid,
        correlation_id: &str,
    ) -> Result<PaymentIntent, PaymentServiceError> {
        get::get(
            self,
            tenant_id,
            environment,
            payment_intent_id,
            actor_key_id,
            correlation_id,
        )
        .await
    }

    pub async fn create_payment_attempt(
        &self,
        command: CreatePaymentAttemptCommand,
    ) -> Result<(PaymentAttemptRecord, bool), PaymentServiceError> {
        attempts::create(self, command).await
    }

    pub async fn list_payment_attempts(
        &self,
        tenant_id: Uuid,
        environment: &str,
        payment_intent_id: Uuid,
        actor_key_id: Uuid,
        correlation_id: &str,
        limit: i32,
    ) -> Result<Vec<PaymentAttemptRecord>, PaymentServiceError> {
        attempts::list(self, tenant_id, environment, payment_intent_id, actor_key_id, correlation_id, limit).await
    }

    pub async fn list_payment_transactions(
        &self,
        tenant_id: Uuid,
        environment: &str,
        payment_intent_id: Uuid,
        actor_key_id: Uuid,
        correlation_id: &str,
        limit: i32,
    ) -> Result<Vec<PaymentTransactionRecord>, PaymentServiceError> {
        transactions::list(self, tenant_id, environment, payment_intent_id, actor_key_id, correlation_id, limit).await
    }

    pub async fn update_payment_intent_status(
        &self,
        command: UpdatePaymentIntentStatusCommand,
    ) -> Result<PaymentIntentStatusUpdate, PaymentServiceError> {
        lifecycle_status::update(self, command).await
    }
}

#[derive(Clone, Debug)]
pub struct CreatePaymentAttemptCommand {
    pub tenant_id: Uuid,
    pub actor_key_id: Uuid,
    pub environment: String,
    pub payment_intent_id: Uuid,
    pub provider_code: String,
    pub provider_account_reference: String,
    pub idempotency_key: String,
    pub correlation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentAttemptRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub environment: String,
    pub payment_intent_id: Uuid,
    pub attempt_number: i32,
    pub provider_code: String,
    pub provider_account_reference: String,
    pub status: String,
    pub aggregate_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentTransactionRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub environment: String,
    pub payment_intent_id: Uuid,
    pub payment_attempt_id: Uuid,
    pub provider_code: String,
    pub provider_account_reference: String,
    pub provider_transaction_reference: String,
    pub amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub aggregate_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct UpdatePaymentIntentStatusCommand {
    pub tenant_id: Uuid,
    pub actor_key_id: Uuid,
    pub environment: String,
    pub payment_intent_id: Uuid,
    pub payment_attempt_id: Uuid,
    pub status: String,
    pub expected_version: i64,
    pub correlation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentIntentStatusUpdate {
    pub intent: PaymentIntent,
    pub aggregate_version: i64,
}
