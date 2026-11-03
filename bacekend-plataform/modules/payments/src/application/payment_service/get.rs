use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::domain::payment_intent::{PaymentIntent, PaymentIntentError, PaymentMethodType};

use super::{PaymentService, PaymentServiceError};

pub(super) async fn get(
    service: &PaymentService,
    tenant_id: Uuid,
    environment: &str,
    payment_intent_id: Uuid,
    actor_key_id: Uuid,
    correlation_id: &str,
) -> Result<PaymentIntent, PaymentServiceError> {
    if !matches!(environment, "test" | "live") {
        return Err(PaymentIntentError::EnvironmentInvalid.into());
    }
    let mut tx = service.pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query(
        "SELECT id,tenant_id,environment,amount_minor,currency,payment_method_type,merchant_reference,status,created_at FROM payments.payment_intents WHERE id=$1 AND tenant_id=$2 AND environment=$3",
    )
    .bind(payment_intent_id)
    .bind(tenant_id)
    .bind(environment)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentIntentError::NotFound)?;
    let intent = PaymentIntent {
        id: row.try_get("id")?,
        tenant_id: row.try_get("tenant_id")?,
        environment: row.try_get("environment")?,
        amount_minor: row.try_get("amount_minor")?,
        currency: row.try_get("currency")?,
        payment_method_type: PaymentMethodType::parse(
            &row.try_get::<String, _>("payment_method_type")?,
        )
        .map_err(|_| PaymentServiceError::SnapshotUnavailable)?,
        merchant_reference: row.try_get("merchant_reference")?,
        status: row.try_get("status")?,
        created_at: row.try_get("created_at")?,
    };
    sqlx::query(
        "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'payment_intent.read',$4,'completed',$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(actor_key_id)
    .bind(payment_intent_id)
    .bind(correlation_id)
    .bind(payvia_observability::current_trace_id())
    .bind(json!({"environment": environment}))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(intent)
}
