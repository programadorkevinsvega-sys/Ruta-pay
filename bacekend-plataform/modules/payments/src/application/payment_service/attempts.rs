use chrono::{DateTime, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgRow, Row};
use uuid::Uuid;

use crate::domain::{
    payment_attempt::PaymentAttempt,
    payment_intent::{PaymentIntent, PaymentIntentError, PaymentMethodType},
    payment_lifecycle::{LifecycleModelError, PaymentEnvironment},
};

use super::{
    CreatePaymentAttemptCommand, PaymentAttemptRecord, PaymentService, PaymentServiceError,
};

pub(super) async fn create(
    service: &PaymentService,
    command: CreatePaymentAttemptCommand,
) -> Result<(PaymentAttemptRecord, bool), PaymentServiceError> {
    PaymentEnvironment::parse(&command.environment)?;
    validate_command(&command)?;
    let request_hash = request_hash(&command)?;
    let mut tx = service.pool.begin().await?;
    set_tenant(&mut tx, command.tenant_id).await?;

    let intent_row = sqlx::query(
        "SELECT id,tenant_id,environment,amount_minor,currency,payment_method_type,merchant_reference,status,created_at FROM payments.payment_intents WHERE id=$1 AND tenant_id=$2 AND environment=$3 FOR UPDATE",
    )
    .bind(command.payment_intent_id)
    .bind(command.tenant_id)
    .bind(&command.environment)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentIntentError::NotFound)?;
    let intent = payment_intent_from_row(&intent_row)?;

    if let Some(row) = sqlx::query(
        "SELECT id,tenant_id,environment,payment_intent_id,attempt_number,provider_code,provider_account_reference,status,aggregate_version,created_at,updated_at,request_hash FROM payments.payment_attempts WHERE tenant_id=$1 AND environment=$2 AND payment_intent_id=$3 AND idempotency_key=$4 FOR UPDATE",
    )
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(command.payment_intent_id)
    .bind(&command.idempotency_key)
    .fetch_optional(&mut *tx)
    .await?
    {
        let stored_hash: Option<Vec<u8>> = row.try_get("request_hash")?;
        if stored_hash.as_deref() != Some(request_hash.as_slice()) {
            return Err(PaymentIntentError::IdempotencyConflict.into());
        }
        let attempt = attempt_from_row(&row)?;
        audit_attempt(
            &mut tx,
            &command,
            attempt.id,
            "payment_attempt.creation_replayed",
            json!({"attempt_number": attempt.attempt_number}),
        )
        .await?;
        tx.commit().await?;
        return Ok((attempt, true));
    }

    let attempt_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM payments.payment_attempts WHERE tenant_id=$1 AND environment=$2 AND payment_intent_id=$3)",
    )
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(command.payment_intent_id)
    .fetch_one(&mut *tx)
    .await?;
    if attempt_exists {
        return Err(PaymentServiceError::AttemptAlreadyExists);
    }

    if intent.status != "created" {
        return Err(PaymentServiceError::StatusUpdateNotAllowed);
    }

    let next_number: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(attempt_number),0)::bigint + 1 FROM payments.payment_attempts WHERE tenant_id=$1 AND environment=$2 AND payment_intent_id=$3",
    )
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(command.payment_intent_id)
    .fetch_one(&mut *tx)
    .await?;
    let attempt_number = i32::try_from(next_number).map_err(|_| {
        PaymentServiceError::Lifecycle(
            crate::domain::payment_lifecycle::LifecycleModelError::AttemptNumberInvalid,
        )
    })?;
    let attempt = PaymentAttempt::new(
        Uuid::new_v4(),
        &intent,
        u32::try_from(attempt_number).map_err(|_| {
            crate::domain::payment_lifecycle::LifecycleModelError::AttemptNumberInvalid
        })?,
        command.provider_code.clone(),
        command.provider_account_reference.clone(),
    )?;

    let row = sqlx::query(
        "INSERT INTO payments.payment_attempts (id,tenant_id,environment,payment_intent_id,attempt_number,provider_code,provider_account_reference,status,aggregate_version,idempotency_key,request_hash) VALUES ($1,$2,$3,$4,$5,$6,$7,'created',1,$8,$9) RETURNING id,tenant_id,environment,payment_intent_id,attempt_number,provider_code,provider_account_reference,status,aggregate_version,created_at,updated_at",
    )
    .bind(attempt.id())
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(command.payment_intent_id)
    .bind(attempt_number)
    .bind(attempt.provider_code())
    .bind(attempt.provider_account_reference())
    .bind(&command.idempotency_key)
    .bind(&request_hash)
    .fetch_one(&mut *tx)
    .await?;
    let record = attempt_from_row(&row)?;

    audit_attempt(
        &mut tx,
        &command,
        record.id,
        "payment_attempt.created",
        json!({
            "attempt_number": record.attempt_number,
            "provider_code": record.provider_code,
            "status": record.status
        }),
    )
    .await?;
    sqlx::query(
        "INSERT INTO payments.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'payment_attempt.created',1,'tenant',$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(record.id)
    .bind(json!({
        "tenant_id": record.tenant_id,
        "environment": record.environment,
        "payment_intent_id": record.payment_intent_id,
        "payment_attempt_id": record.id,
        "attempt_number": record.attempt_number,
        "provider_code": record.provider_code,
        "status": record.status
    }))
    .bind(payvia_observability::current_traceparent())
    .bind(&command.correlation_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((record, false))
}

pub(super) async fn list(
    service: &PaymentService,
    tenant_id: Uuid,
    environment: &str,
    payment_intent_id: Uuid,
    actor_key_id: Uuid,
    correlation_id: &str,
    limit: i32,
) -> Result<Vec<PaymentAttemptRecord>, PaymentServiceError> {
    PaymentEnvironment::parse(environment)?;
    validate_limit(limit)?;
    let mut tx = service.pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    ensure_intent(&mut tx, tenant_id, environment, payment_intent_id).await?;
    let rows = sqlx::query(
        "SELECT id,tenant_id,environment,payment_intent_id,attempt_number,provider_code,provider_account_reference,status,aggregate_version,created_at,updated_at FROM payments.payment_attempts WHERE tenant_id=$1 AND environment=$2 AND payment_intent_id=$3 ORDER BY attempt_number DESC LIMIT $4",
    )
    .bind(tenant_id)
    .bind(environment)
    .bind(payment_intent_id)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'payment_attempts.read',$4,'completed',$5,$6,$7)",
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
        .map(attempt_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    tx.commit().await?;
    Ok(records)
}

fn validate_command(command: &CreatePaymentAttemptCommand) -> Result<(), PaymentServiceError> {
    if command.provider_code.len() < 2
        || command.provider_code.len() > 32
        || !command
            .provider_code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(LifecycleModelError::ProviderCodeInvalid.into());
    }
    if command.provider_account_reference.is_empty()
        || command.provider_account_reference.len() > 128
        || command
            .provider_account_reference
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(LifecycleModelError::ProviderReferenceInvalid.into());
    }
    if command.idempotency_key.is_empty()
        || command.idempotency_key.len() > 128
        || !command
            .idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
        || command.correlation_id.is_empty()
        || command.correlation_id.len() > 128
        || command
            .correlation_id
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(LifecycleModelError::IdentifierInvalid.into());
    }
    Ok(())
}

fn request_hash(command: &CreatePaymentAttemptCommand) -> Result<Vec<u8>, PaymentServiceError> {
    let canonical = serde_json::to_vec(&json!({
        "tenant_id": command.tenant_id,
        "environment": command.environment,
        "payment_intent_id": command.payment_intent_id,
        "provider_code": command.provider_code,
        "provider_account_reference": command.provider_account_reference
    }))?;
    Ok(Sha256::digest(canonical).to_vec())
}

async fn set_tenant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn ensure_intent(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    environment: &str,
    payment_intent_id: Uuid,
) -> Result<(), PaymentServiceError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM payments.payment_intents WHERE id=$1 AND tenant_id=$2 AND environment=$3)",
    )
    .bind(payment_intent_id)
    .bind(tenant_id)
    .bind(environment)
    .fetch_one(&mut **tx)
    .await?;
    if !exists {
        return Err(PaymentIntentError::NotFound.into());
    }
    Ok(())
}

async fn audit_attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    command: &CreatePaymentAttemptCommand,
    attempt_id: Uuid,
    action: &str,
    details: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,$4,$5,'completed',$6,$7,$8)",
    )
    .bind(Uuid::new_v4())
    .bind(command.tenant_id)
    .bind(command.actor_key_id)
    .bind(action)
    .bind(attempt_id)
    .bind(&command.correlation_id)
    .bind(payvia_observability::current_trace_id())
    .bind(details)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn payment_intent_from_row(row: &PgRow) -> Result<PaymentIntent, PaymentServiceError> {
    Ok(PaymentIntent {
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
    })
}

fn attempt_from_row(row: &PgRow) -> Result<PaymentAttemptRecord, sqlx::Error> {
    Ok(PaymentAttemptRecord {
        id: row.try_get("id")?,
        tenant_id: row.try_get("tenant_id")?,
        environment: row.try_get("environment")?,
        payment_intent_id: row.try_get("payment_intent_id")?,
        attempt_number: row.try_get("attempt_number")?,
        provider_code: row.try_get("provider_code")?,
        provider_account_reference: row.try_get("provider_account_reference")?,
        status: row.try_get("status")?,
        aggregate_version: row.try_get("aggregate_version")?,
        created_at: row.try_get::<DateTime<Utc>, _>("created_at")?,
        updated_at: row.try_get::<DateTime<Utc>, _>("updated_at")?,
    })
}

fn validate_limit(limit: i32) -> Result<(), PaymentServiceError> {
    if !(1..=100).contains(&limit) {
        return Err(PaymentServiceError::InvalidPageSize);
    }
    Ok(())
}
