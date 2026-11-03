use sqlx::Row;
use uuid::Uuid;

use crate::domain::payment_intent::{PaymentIntent, PaymentIntentError, PaymentMethodType};

use super::{PaymentService, PaymentServiceError};

pub(super) async fn get(
    service: &PaymentService,
    tenant_id: Uuid,
    payment_intent_id: Uuid,
) -> Result<PaymentIntent, PaymentServiceError> {
    let mut tx = service.pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query(
        "SELECT id,tenant_id,environment,amount_minor,currency,payment_method_type,merchant_reference,status,created_at FROM payments.payment_intents WHERE id=$1 AND tenant_id=$2",
    )
    .bind(payment_intent_id)
    .bind(tenant_id)
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
    tx.commit().await?;
    Ok(intent)
}
