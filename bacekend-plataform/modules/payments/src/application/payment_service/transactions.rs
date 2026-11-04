use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{postgres::PgRow, Row};
use uuid::Uuid;

use crate::domain::{payment_intent::PaymentIntentError, payment_lifecycle::PaymentEnvironment};

use super::{PaymentService, PaymentServiceError, PaymentTransactionRecord};

pub(super) async fn list(
    service: &PaymentService,
    tenant_id: Uuid,
    environment: &str,
    payment_intent_id: Uuid,
    actor_key_id: Uuid,
    correlation_id: &str,
    limit: i32,
) -> Result<Vec<PaymentTransactionRecord>, PaymentServiceError> {
    PaymentEnvironment::parse(environment)?;
    if !(1..=100).contains(&limit) {
        return Err(PaymentServiceError::InvalidPageSize);
    }
    let mut tx = service.pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM payments.payment_intents WHERE id=$1 AND tenant_id=$2 AND environment=$3)",
    )
    .bind(payment_intent_id)
    .bind(tenant_id)
    .bind(environment)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(PaymentIntentError::NotFound.into());
    }
    let rows = sqlx::query(
        "SELECT id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_transaction_reference,amount_minor,currency,status,aggregate_version,created_at,updated_at FROM payments.payment_transactions WHERE tenant_id=$1 AND environment=$2 AND payment_intent_id=$3 ORDER BY created_at,id LIMIT $4",
    )
    .bind(tenant_id)
    .bind(environment)
    .bind(payment_intent_id)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'payment_transactions.read',$4,'completed',$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(actor_key_id)
    .bind(payment_intent_id)
    .bind(correlation_id)
    .bind(payvia_observability::current_trace_id())
    .bind(json!({"environment": environment, "limit": limit}))
    .execute(&mut *tx)
    .await?;
    let records = rows
        .iter()
        .map(transaction_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    tx.commit().await?;
    Ok(records)
}

fn transaction_from_row(row: &PgRow) -> Result<PaymentTransactionRecord, sqlx::Error> {
    Ok(PaymentTransactionRecord {
        id: row.try_get("id")?,
        tenant_id: row.try_get("tenant_id")?,
        environment: row.try_get("environment")?,
        payment_intent_id: row.try_get("payment_intent_id")?,
        payment_attempt_id: row.try_get("payment_attempt_id")?,
        provider_code: row.try_get("provider_code")?,
        provider_account_reference: row.try_get("provider_account_reference")?,
        provider_transaction_reference: row.try_get("provider_transaction_reference")?,
        amount_minor: row.try_get("amount_minor")?,
        currency: row.try_get("currency")?,
        status: row.try_get("status")?,
        aggregate_version: row.try_get("aggregate_version")?,
        created_at: row.try_get::<DateTime<Utc>, _>("created_at")?,
        updated_at: row.try_get::<DateTime<Utc>, _>("updated_at")?,
    })
}
