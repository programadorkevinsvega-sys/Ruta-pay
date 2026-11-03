use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::domain::payment_intent::{PaymentIntent, validate_create_input};

use super::{CreatePaymentIntentCommand, PaymentService, PaymentServiceError, snapshot};

const IDEMPOTENCY_OPERATION: &str = "payment_intent.create.v1";

#[derive(Serialize)]
struct RequestFingerprint<'a> {
    tenant_id: String,
    environment: &'a str,
    amount_minor: i64,
    currency: &'a str,
    payment_method_type: &'a str,
    merchant_reference: Option<&'a str>,
}

pub(super) async fn create(
    service: &PaymentService,
    command: CreatePaymentIntentCommand,
) -> Result<(PaymentIntent, bool), PaymentServiceError> {
    let method = validate_create_input(
        &command.environment,
        command.amount_minor,
        &command.currency,
        &command.payment_method_type,
        command.merchant_reference.as_deref(),
        &command.idempotency_key,
    )?;
    let fingerprint = request_fingerprint(&command)?;
    let mut tx = service.pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(command.tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let idempotency_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payments.idempotency_records (id,tenant_id,environment,operation,idempotency_key,request_hash) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (tenant_id,environment,operation,idempotency_key) DO NOTHING RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(IDEMPOTENCY_OPERATION)
    .bind(&command.idempotency_key)
    .bind(&fingerprint)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(idempotency_id) = idempotency_id else {
        let row = sqlx::query(
            "SELECT request_hash,response_snapshot FROM payments.idempotency_records WHERE tenant_id=$1 AND environment=$2 AND operation=$3 AND idempotency_key=$4 FOR UPDATE",
        )
        .bind(command.tenant_id)
        .bind(&command.environment)
        .bind(IDEMPOTENCY_OPERATION)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PaymentServiceError::SnapshotUnavailable)?;
        let stored_hash: Vec<u8> = row.try_get("request_hash")?;
        if stored_hash != fingerprint {
            return Err(
                crate::domain::payment_intent::PaymentIntentError::IdempotencyConflict.into(),
            );
        }
        let response: Option<serde_json::Value> = row.try_get("response_snapshot")?;
        let intent = snapshot::decode(response.ok_or(PaymentServiceError::SnapshotUnavailable)?)?;
        sqlx::query(
            "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'payment_intent.creation_replayed',$4,'completed',$5,$6,$7)",
        )
        .bind(Uuid::new_v4())
        .bind(command.tenant_id)
        .bind(command.actor_key_id)
        .bind(intent.id)
        .bind(&command.correlation_id)
        .bind(payvia_observability::current_trace_id())
        .bind(json!({"environment": command.environment, "idempotency_key": command.idempotency_key}))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok((intent, true));
    };

    let intent_id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO payments.payment_intents (id,tenant_id,environment,amount_minor,currency,payment_method_type,merchant_reference,status,aggregate_version) VALUES ($1,$2,$3,$4,$5,$6,$7,'created',1) RETURNING created_at",
    )
    .bind(intent_id)
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(command.amount_minor)
    .bind(&command.currency)
    .bind(method.as_str())
    .bind(&command.merchant_reference)
    .fetch_one(&mut *tx)
    .await?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let intent = PaymentIntent {
        id: intent_id,
        tenant_id: command.tenant_id,
        environment: command.environment.clone(),
        amount_minor: command.amount_minor,
        currency: command.currency.clone(),
        payment_method_type: method,
        merchant_reference: command.merchant_reference.clone(),
        status: "created".to_owned(),
        created_at,
    };
    sqlx::query(
        "UPDATE payments.idempotency_records SET payment_intent_id=$2,response_snapshot=$3 WHERE id=$1",
    )
    .bind(idempotency_id)
    .bind(intent_id)
    .bind(snapshot::encode(&intent))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'payment_intent.created',$4,'completed',$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(command.tenant_id)
    .bind(command.actor_key_id)
    .bind(intent_id)
    .bind(&command.correlation_id)
    .bind(payvia_observability::current_trace_id())
    .bind(json!({
        "environment": command.environment,
        "amount_minor": command.amount_minor,
        "currency": command.currency,
        "payment_method_type": method.as_str()
    }))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO payments.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'payment_intent.created',1,'tenant',$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(intent_id)
    .bind(json!({
        "tenant_id": command.tenant_id,
        "payment_intent_id": intent_id,
        "environment": command.environment,
        "amount_minor": command.amount_minor,
        "currency": command.currency,
        "payment_method_type": method.as_str(),
        "status": "created"
    }))
    .bind(payvia_observability::current_traceparent())
    .bind(&command.correlation_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(
        event.name = "payment_intent.created",
        component = "payments",
        operation = "create_payment_intent",
        payment_intent_id = %intent_id,
        tenant_id = %command.tenant_id,
        environment = %command.environment,
        correlation_id = %command.correlation_id
    );
    Ok((intent, false))
}

fn request_fingerprint(
    command: &CreatePaymentIntentCommand,
) -> Result<Vec<u8>, PaymentServiceError> {
    let canonical = RequestFingerprint {
        tenant_id: command.tenant_id.to_string(),
        environment: &command.environment,
        amount_minor: command.amount_minor,
        currency: &command.currency,
        payment_method_type: &command.payment_method_type,
        merchant_reference: command.merchant_reference.as_deref(),
    };
    let bytes = serde_json::to_vec(&canonical)?;
    Ok(Sha256::digest(bytes).to_vec())
}
