use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

mod support;
use support::*;

use crate::domain::payment_lifecycle::{
    PaymentAttemptStatus, PaymentIntentStatus, PaymentTransactionStatus, TransitionAuthority,
    TransitionEffect, ensure_provider_event_is_newer,
};

use super::{
    GatewayEventDisposition, PaymentService, PaymentServiceError,
    gateway_event_auth::verify_signed_event,
};

#[derive(Deserialize)]
struct ProviderEvent {
    schema_version: u32,
    event_id: Uuid,
    provider_code: String,
    provider_account_reference: String,
    provider_event_id: String,
    tenant_id: Uuid,
    environment: String,
    payment_attempt_id: Uuid,
    provider_transaction_reference: String,
    status: String,
    amount_minor: i64,
    currency: String,
    occurred_at_unix_millis: i64,
    correlation_id: String,
    payload_sha256: String,
}

pub(super) async fn ingest(
    service: &PaymentService,
    payload: &[u8],
    signing_key: &[u8],
    expected_key_id: &str,
) -> Result<GatewayEventDisposition, PaymentServiceError> {
    let value = verify_signed_event(payload, signing_key, expected_key_id)?;
    let event: ProviderEvent = serde_json::from_value(value)?;
    let occurred_at = validate_event(&event)?;

    let mut tx = service.pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(event.tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let attempt = sqlx::query(
        "SELECT a.payment_intent_id,a.status AS attempt_status,a.aggregate_version AS attempt_version,a.last_event_at AS attempt_last_event_at,a.provider_code,a.provider_account_reference,i.status AS intent_status,i.aggregate_version AS intent_version,i.amount_minor,i.currency FROM payments.payment_attempts a JOIN payments.payment_intents i ON i.id=a.payment_intent_id AND i.tenant_id=a.tenant_id AND i.environment=a.environment WHERE a.id=$1 AND a.tenant_id=$2 AND a.environment=$3 FOR UPDATE OF a,i",
    )
    .bind(event.payment_attempt_id)
    .bind(event.tenant_id)
    .bind(&event.environment)
    .fetch_optional(&mut *tx)
    .await?;
    let attempt = match attempt {
        Some(row) => row,
        None => {
            tx.rollback().await?;
            return Err(PaymentServiceError::ProviderEventInvalid);
        }
    };
    let payment_intent_id: Uuid = attempt.try_get("payment_intent_id")?;
    let attempt_provider: String = attempt.try_get("provider_code")?;
    let attempt_account: String = attempt.try_get("provider_account_reference")?;
    let amount_minor: i64 = attempt.try_get("amount_minor")?;
    let currency: String = attempt.try_get("currency")?;
    if attempt_provider != event.provider_code
        || attempt_account != event.provider_account_reference
        || amount_minor != event.amount_minor
        || currency.trim() != event.currency
    {
        tx.rollback().await?;
        return Err(PaymentServiceError::ProviderEventInvalid);
    }
    if attempt.try_get::<String, _>("attempt_status")? == "created" {
        tx.rollback().await?;
        return Err(PaymentServiceError::ProviderEventInvalid);
    }

    let payload_hash =
        decode_sha256(&event.payload_sha256).ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let inbox_id = Uuid::new_v4();
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payments.provider_event_inbox (inbox_id,event_id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_event_id,provider_transaction_reference,normalized_status,schema_version,occurred_at,payload_sha256,correlation_id,producer_identity) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,1,$12,$13,$14,'gateway-pasarela:wompi') ON CONFLICT DO NOTHING RETURNING inbox_id",
    )
    .bind(inbox_id)
    .bind(event.event_id)
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(payment_intent_id)
    .bind(event.payment_attempt_id)
    .bind(&event.provider_code)
    .bind(&event.provider_account_reference)
    .bind(&event.provider_event_id)
    .bind(&event.provider_transaction_reference)
    .bind(&event.status)
    .bind(occurred_at)
    .bind(payload_hash.as_slice())
    .bind(&event.correlation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT inbox_id FROM payments.provider_event_inbox WHERE tenant_id=$1 AND environment=$2 AND provider_code=$3 AND provider_account_reference=$4 AND provider_event_id=$5",
        )
        .bind(event.tenant_id)
        .bind(&event.environment)
        .bind(&event.provider_code)
        .bind(&event.provider_account_reference)
        .bind(&event.provider_event_id)
        .fetch_optional(&mut *tx)
        .await?;
        let existing = existing.ok_or(PaymentServiceError::ProviderEventInvalid)?;
        insert_delivery(&mut tx, &event, existing, "duplicate").await?;
        tx.commit().await?;
        return Ok(GatewayEventDisposition::Duplicate);
    }
    insert_delivery(&mut tx, &event, inbox_id, "accepted").await?;

    let current_attempt: String = attempt.try_get("attempt_status")?;
    let current_intent: String = attempt.try_get("intent_status")?;
    let attempt_version: i64 = attempt.try_get("attempt_version")?;
    let intent_version: i64 = attempt.try_get("intent_version")?;
    let attempt_last_event: Option<DateTime<Utc>> = attempt.try_get("attempt_last_event_at")?;
    let intent_target = intent_target(&event.status)?;
    let attempt_target = attempt_target(&event.status)?;
    let unknown_outcome =
        current_attempt == "outcome_unknown" || current_intent == "outcome_unknown";
    let late_event = ensure_provider_event_is_newer(attempt_last_event, occurred_at).is_err();
    let attempt_transition_conflict = transition_attempt(&current_attempt, attempt_target).is_err();
    let intent_transition_conflict =
        intent_target.is_some_and(|target| transition_intent(&current_intent, target).is_err());
    let needs_reconciliation =
        unknown_outcome || late_event || attempt_transition_conflict || intent_transition_conflict;

    if needs_reconciliation {
        let reason = if unknown_outcome {
            "outcome_unknown"
        } else if late_event {
            "late_event"
        } else {
            "terminal_conflict"
        };
        mark_inbox(&mut tx, inbox_id, "reconciliation_required").await?;
        request_reconciliation(&mut tx, &event, payment_intent_id, reason).await?;
        tx.commit().await?;
        return Ok(GatewayEventDisposition::ReconciliationRequired);
    }

    let target_transaction = transaction_target(&event.status)?;
    let transaction_row = sqlx::query(
        "SELECT id,payment_attempt_id,status,aggregate_version,last_event_at FROM payments.payment_transactions WHERE tenant_id=$1 AND environment=$2 AND provider_code=$3 AND provider_account_reference=$4 AND provider_transaction_reference=$5 FOR UPDATE",
    )
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(&event.provider_code)
    .bind(&event.provider_account_reference)
    .bind(&event.provider_transaction_reference)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = &transaction_row {
        let existing_attempt: Uuid = row.try_get("payment_attempt_id")?;
        let current: String = row.try_get("status")?;
        let last_event: Option<DateTime<Utc>> = row.try_get("last_event_at")?;
        if existing_attempt != event.payment_attempt_id
            || ensure_provider_event_is_newer(last_event, occurred_at).is_err()
            || transition_transaction(&current, target_transaction).is_err()
        {
            mark_inbox(&mut tx, inbox_id, "reconciliation_required").await?;
            request_reconciliation(&mut tx, &event, payment_intent_id, "terminal_conflict").await?;
            tx.commit().await?;
            return Ok(GatewayEventDisposition::ReconciliationRequired);
        }
    }

    let attempt_status = attempt_status_text(attempt_target);
    let attempt_next_version: i64 = sqlx::query_scalar(
        "UPDATE payments.payment_attempts SET status=$2,aggregate_version=aggregate_version+1,last_event_at=$3,updated_at=now() WHERE id=$1 AND aggregate_version=$4 RETURNING aggregate_version",
    )
    .bind(event.payment_attempt_id)
    .bind(attempt_status)
    .bind(occurred_at)
    .bind(attempt_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentServiceError::VersionConflict)?;

    let mut intent_next_version = intent_version;
    if let Some(target) = intent_target
        && intent_status_text(target) != current_intent
    {
        intent_next_version = sqlx::query_scalar(
            "UPDATE payments.payment_intents SET status=$2,aggregate_version=aggregate_version+1,last_event_at=$3,updated_at=now() WHERE id=$1 AND tenant_id=$4 AND environment=$5 AND aggregate_version=$6 RETURNING aggregate_version",
        )
        .bind(payment_intent_id)
        .bind(intent_status_text(target))
        .bind(occurred_at)
        .bind(event.tenant_id)
        .bind(&event.environment)
        .bind(intent_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PaymentServiceError::VersionConflict)?;
    }

    let transaction_id = match transaction_row {
        Some(row) => {
            let id: Uuid = row.try_get("id")?;
            let version: i64 = row.try_get("aggregate_version")?;
            sqlx::query(
                "UPDATE payments.payment_transactions SET status=$2,aggregate_version=aggregate_version+1,last_event_at=$3,updated_at=now() WHERE id=$1 AND aggregate_version=$4",
            )
            .bind(id)
            .bind(transaction_status_text(target_transaction))
            .bind(occurred_at)
            .bind(version)
            .execute(&mut *tx)
            .await?;
            id
        }
        None => {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO payments.payment_transactions (id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_transaction_reference,amount_minor,currency,status,aggregate_version,last_event_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,1,$12)",
            )
            .bind(id)
            .bind(event.tenant_id)
            .bind(&event.environment)
            .bind(payment_intent_id)
            .bind(event.payment_attempt_id)
            .bind(&event.provider_code)
            .bind(&event.provider_account_reference)
            .bind(&event.provider_transaction_reference)
            .bind(event.amount_minor)
            .bind(&event.currency)
            .bind(transaction_status_text(target_transaction))
            .bind(occurred_at)
            .execute(&mut *tx)
            .await?;
            id
        }
    };

    audit_service_event(
        &mut tx,
        &event,
        event.payment_attempt_id,
        "payment_attempt.provider_event_applied",
        json!({"status": attempt_status, "aggregate_version": attempt_next_version}),
    )
    .await?;
    audit_service_event(
        &mut tx,
        &event,
        transaction_id,
        "payment_transaction.provider_event_applied",
        json!({"status": transaction_status_text(target_transaction)}),
    )
    .await?;
    insert_outbox(
        &mut tx,
        event.payment_attempt_id,
        attempt_next_version,
        "payment_attempt.status_changed",
        json!({"tenant_id":event.tenant_id,"environment":event.environment,"payment_intent_id":payment_intent_id,"attempt_id":event.payment_attempt_id,"status":attempt_status,"aggregate_version":attempt_next_version}),
        &event.correlation_id,
    )
    .await?;
    if intent_next_version > intent_version {
        audit_service_event(
            &mut tx,
            &event,
            payment_intent_id,
            "payment_intent.provider_event_applied",
            json!({"status":intent_target.map(intent_status_text),"aggregate_version":intent_next_version}),
        )
        .await?;
        insert_outbox(
            &mut tx,
            payment_intent_id,
            intent_next_version,
            "payment_intent.status_changed",
            json!({"tenant_id":event.tenant_id,"environment":event.environment,"payment_intent_id":payment_intent_id,"status":intent_target.map(intent_status_text),"aggregate_version":intent_next_version}),
            &event.correlation_id,
        )
        .await?;
    }
    audit_service_event(
        &mut tx,
        &event,
        transaction_id,
        "payment_transaction.status_changed",
        json!({"status":transaction_status_text(target_transaction)}),
    )
    .await?;
    mark_inbox(&mut tx, inbox_id, "applied").await?;
    tx.commit().await?;
    Ok(GatewayEventDisposition::Applied)
}
