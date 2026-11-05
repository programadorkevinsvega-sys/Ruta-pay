use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

mod support;
use support::*;

use crate::domain::payment_lifecycle::{
    PaymentAttemptStatus, PaymentIntentStatus, PaymentTransactionStatus, TransitionAuthority,
    TransitionEffect,
};

use super::{
    GatewayEventDisposition, PaymentService, PaymentServiceError,
    gateway_event_auth::verify_signed_event,
};

#[derive(Deserialize)]
struct ReconciliationResult {
    schema_version: u32,
    event_id: Uuid,
    event_type: String,
    provider_code: String,
    provider_account_reference: String,
    tenant_id: Uuid,
    environment: String,
    payment_intent_id: Uuid,
    payment_attempt_id: Uuid,
    reconciliation_case_id: Uuid,
    provider_transaction_reference: Option<String>,
    outcome: String,
    status: Option<String>,
    amount_minor: Option<i64>,
    currency: Option<String>,
    evidence_sha256: Option<String>,
    safe_error_code: Option<String>,
    correlation_id: String,
    occurred_at_unix_millis: i64,
}

pub(super) async fn apply(
    service: &PaymentService,
    payload: &[u8],
    signing_key: &[u8],
    expected_key_id: &str,
) -> Result<GatewayEventDisposition, PaymentServiceError> {
    let value = verify_signed_event(payload, signing_key, expected_key_id)?;
    let mut event: ReconciliationResult = serde_json::from_value(value)?;
    let occurred_at = validate_result(&event)?;
    let mut tx = service.pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(event.tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let case = sqlx::query(
        "SELECT status,payment_intent_id,payment_attempt_id,result_status FROM payments.payment_reconciliation_cases WHERE id=$1 AND tenant_id=$2 AND environment=$3 FOR UPDATE",
    )
    .bind(event.reconciliation_case_id)
    .bind(event.tenant_id)
    .bind(&event.environment)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    if case.try_get::<Uuid, _>("payment_intent_id")? != event.payment_intent_id
        || case.try_get::<Uuid, _>("payment_attempt_id")? != event.payment_attempt_id
    {
        tx.rollback().await?;
        return Err(PaymentServiceError::ProviderEventInvalid);
    }
    let case_status: String = case.try_get("status")?;
    if case_status == "resolved" {
        let stored_status: Option<String> = case.try_get("result_status")?;
        if stored_status.as_deref() == event.status.as_deref() {
            tx.commit().await?;
            return Ok(GatewayEventDisposition::Duplicate);
        }
        tx.rollback().await?;
        return Err(PaymentServiceError::ProviderEventInvalid);
    }

    if event.outcome == "manual_review" {
        let recorded = record_manual_review(&mut tx, &event).await?;
        tx.commit().await?;
        return Ok(if recorded {
            GatewayEventDisposition::ManualReview
        } else {
            GatewayEventDisposition::Duplicate
        });
    }

    let status = event
        .status
        .as_deref()
        .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let provider_reference = event
        .provider_transaction_reference
        .as_deref()
        .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let amount_minor = event
        .amount_minor
        .filter(|amount| *amount > 0)
        .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let currency = event
        .currency
        .as_deref()
        .filter(|value| value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase()))
        .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let evidence = decode_sha256(
        event
            .evidence_sha256
            .as_deref()
            .ok_or(PaymentServiceError::ProviderEventInvalid)?,
    )
    .ok_or(PaymentServiceError::ProviderEventInvalid)?;

    let aggregate = sqlx::query(
        "SELECT a.status AS attempt_status,a.aggregate_version AS attempt_version,a.provider_code,a.provider_account_reference,i.status AS intent_status,i.aggregate_version AS intent_version,i.amount_minor,i.currency FROM payments.payment_attempts a JOIN payments.payment_intents i ON i.id=a.payment_intent_id AND i.tenant_id=a.tenant_id AND i.environment=a.environment WHERE a.id=$1 AND a.tenant_id=$2 AND a.environment=$3 AND a.payment_intent_id=$4 FOR UPDATE OF a,i",
    )
    .bind(event.payment_attempt_id)
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(event.payment_intent_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let attempt_status: String = aggregate.try_get("attempt_status")?;
    let intent_status: String = aggregate.try_get("intent_status")?;
    let attempt_version: i64 = aggregate.try_get("attempt_version")?;
    let intent_version: i64 = aggregate.try_get("intent_version")?;
    let provider_code: String = aggregate.try_get("provider_code")?;
    let account_reference: String = aggregate.try_get("provider_account_reference")?;
    let expected_amount: i64 = aggregate.try_get("amount_minor")?;
    let expected_currency: String = aggregate.try_get("currency")?;
    if provider_code != event.provider_code
        || account_reference != event.provider_account_reference
        || expected_amount != amount_minor
        || expected_currency.trim() != currency
    {
        tx.rollback().await?;
        return Err(PaymentServiceError::ProviderEventInvalid);
    }

    let target_attempt = parse_attempt_target(status)?;
    let current_attempt = parse_attempt_status(&attempt_status)?;
    if current_attempt == PaymentAttemptStatus::Created
        || current_attempt == PaymentAttemptStatus::Authorized && status == "failed"
    {
        return record_conflict(&mut tx, &mut event, "provider_attempt_state_conflict").await;
    }
    let attempt_effect =
        match current_attempt.transition_to(target_attempt, TransitionAuthority::Reconciliation) {
        Ok(effect) => effect,
        Err(_) => {
            return record_conflict(&mut tx, &mut event, "provider_attempt_terminal_conflict")
                .await;
        }
    };
    let target_intent = if status == "succeeded" {
        PaymentIntentStatus::Succeeded
    } else {
        PaymentIntentStatus::Processing
    };
    let current_intent = parse_intent_status(&intent_status)?;
    if current_intent == PaymentIntentStatus::Authorized && status == "failed" {
        return record_conflict(&mut tx, &mut event, "provider_intent_state_conflict").await;
    }
    if current_intent == PaymentIntentStatus::Created {
        return record_conflict(&mut tx, &mut event, "provider_intent_not_submitted").await;
    }
    let intent_effect = match current_intent
        .transition_to(target_intent, TransitionAuthority::Reconciliation)
    {
        Ok(effect) => effect,
        Err(_) => {
            return record_conflict(&mut tx, &mut event, "provider_intent_terminal_conflict").await;
        }
    };

    let target_transaction = parse_transaction_target(status)?;
    let transaction = sqlx::query(
        "SELECT id,payment_attempt_id,status,aggregate_version FROM payments.payment_transactions WHERE tenant_id=$1 AND environment=$2 AND provider_code=$3 AND provider_account_reference=$4 AND provider_transaction_reference=$5 FOR UPDATE",
    )
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(&event.provider_code)
    .bind(&event.provider_account_reference)
    .bind(provider_reference)
    .fetch_optional(&mut *tx)
    .await?;
    let (transaction_id, transaction_version, transaction_effect) = match transaction {
        Some(row) => {
            if row.try_get::<Uuid, _>("payment_attempt_id")? != event.payment_attempt_id {
                tx.rollback().await?;
                return Err(PaymentServiceError::ProviderEventInvalid);
            }
            let id: Uuid = row.try_get("id")?;
            let current = parse_transaction_status(&row.try_get::<String, _>("status")?)?;
            if current == PaymentTransactionStatus::Authorized && status == "failed" {
                return record_conflict(&mut tx, &mut event, "provider_transaction_state_conflict")
                    .await;
            }
            let effect = match current
                .transition_to(target_transaction, TransitionAuthority::Reconciliation)
            {
                Ok(effect) => effect,
                Err(_) => {
                    return record_conflict(
                        &mut tx,
                        &mut event,
                        "provider_transaction_terminal_conflict",
                    )
                    .await;
                }
            };
            let version: i64 = row.try_get("aggregate_version")?;
            (id, Some(version), effect)
        }
        None => (Uuid::new_v4(), None, TransitionEffect::Apply),
    };

    let next_attempt_number: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(attempt_number),0)+1 FROM payments.payment_reconciliation_attempts WHERE tenant_id=$1 AND environment=$2 AND reconciliation_case_id=$3",
    )
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(event.reconciliation_case_id)
    .fetch_one(&mut *tx)
    .await?;
    let recorded_result: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO payments.payment_reconciliation_attempts (id,tenant_id,environment,reconciliation_case_id,attempt_number,result_status,provider_transaction_reference,evidence_sha256,queried_at,completed_at,result_event_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9,$10) ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(event.reconciliation_case_id)
    .bind(next_attempt_number)
    .bind(status)
    .bind(provider_reference)
    .bind(evidence.as_slice())
    .bind(occurred_at)
    .bind(event.event_id)
    .fetch_optional(&mut *tx)
    .await?;
    if recorded_result.is_none() {
        tx.commit().await?;
        return Ok(GatewayEventDisposition::Duplicate);
    }

    let attempt_next_version = if attempt_effect == TransitionEffect::Apply {
        sqlx::query_scalar(
            "UPDATE payments.payment_attempts SET status=$2,aggregate_version=aggregate_version+1,last_event_at=$3,updated_at=now() WHERE id=$1 AND aggregate_version=$4 RETURNING aggregate_version",
        )
        .bind(event.payment_attempt_id)
        .bind(attempt_status_text(target_attempt))
        .bind(occurred_at)
        .bind(attempt_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PaymentServiceError::VersionConflict)?
    } else {
        attempt_version
    };
    let intent_next_version = if intent_effect == TransitionEffect::Apply {
        sqlx::query_scalar(
            "UPDATE payments.payment_intents SET status=$2,aggregate_version=aggregate_version+1,last_event_at=$3,updated_at=now() WHERE id=$1 AND tenant_id=$4 AND environment=$5 AND aggregate_version=$6 RETURNING aggregate_version",
        )
        .bind(event.payment_intent_id)
        .bind(intent_status_text(target_intent))
        .bind(occurred_at)
        .bind(event.tenant_id)
        .bind(&event.environment)
        .bind(intent_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PaymentServiceError::VersionConflict)?
    } else {
        intent_version
    };
    if transaction_effect == TransitionEffect::Apply {
        if let Some(version) = transaction_version {
            sqlx::query(
                "UPDATE payments.payment_transactions SET status=$2,aggregate_version=aggregate_version+1,last_event_at=$3,updated_at=now() WHERE id=$1 AND aggregate_version=$4",
            )
            .bind(transaction_id)
            .bind(transaction_status_text(target_transaction))
            .bind(occurred_at)
            .bind(version)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO payments.payment_transactions (id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_transaction_reference,amount_minor,currency,status,aggregate_version,last_event_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,1,$12)",
            )
            .bind(transaction_id)
            .bind(event.tenant_id)
            .bind(&event.environment)
            .bind(event.payment_intent_id)
            .bind(event.payment_attempt_id)
            .bind(&event.provider_code)
            .bind(&event.provider_account_reference)
            .bind(provider_reference)
            .bind(amount_minor)
            .bind(currency)
            .bind(transaction_status_text(target_transaction))
            .bind(occurred_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if attempt_effect == TransitionEffect::Apply {
        audit_result(
            &mut tx,
            &event,
            event.payment_attempt_id,
            "payment_attempt.reconciled",
            json!({"status":attempt_status_text(target_attempt),"aggregate_version":attempt_next_version}),
        )
        .await?;
        insert_outbox(
            &mut tx,
            event.payment_attempt_id,
            attempt_next_version,
            "payment_attempt.status_changed",
            json!({"tenant_id":event.tenant_id,"environment":event.environment,"payment_intent_id":event.payment_intent_id,"attempt_id":event.payment_attempt_id,"status":attempt_status_text(target_attempt),"aggregate_version":attempt_next_version}),
            &event.correlation_id,
        )
        .await?;
    }
    if intent_effect == TransitionEffect::Apply {
        audit_result(
            &mut tx,
            &event,
            event.payment_intent_id,
            "payment_intent.reconciled",
            json!({"status":intent_status_text(target_intent),"aggregate_version":intent_next_version}),
        )
        .await?;
        insert_outbox(
            &mut tx,
            event.payment_intent_id,
            intent_next_version,
            "payment_intent.status_changed",
            json!({"tenant_id":event.tenant_id,"environment":event.environment,"payment_intent_id":event.payment_intent_id,"status":intent_status_text(target_intent),"aggregate_version":intent_next_version}),
            &event.correlation_id,
        )
        .await?;
    }
    if transaction_effect == TransitionEffect::Apply {
        audit_result(
            &mut tx,
            &event,
            transaction_id,
            "payment_transaction.reconciled",
            json!({"status":transaction_status_text(target_transaction)}),
        )
        .await?;
    }
    sqlx::query(
        "UPDATE payments.payment_reconciliation_cases SET status='resolved',payment_transaction_id=$2,attempts=attempts+1,result_status=$3,lease_token=NULL,lease_until=NULL,last_error_code=NULL,resolved_at=$4,updated_at=$4 WHERE id=$1",
    )
    .bind(event.reconciliation_case_id)
    .bind(transaction_id)
    .bind(status)
    .bind(occurred_at)
    .execute(&mut *tx)
    .await?;
    let case_outbox_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(aggregate_version),0)+1 FROM payments.outbox_events WHERE aggregate_id=$1",
    )
    .bind(event.reconciliation_case_id)
    .fetch_one(&mut *tx)
    .await?;
    insert_outbox(
        &mut tx,
        event.reconciliation_case_id,
        case_outbox_version,
        "payment.reconciliation.resolved",
        json!({"tenant_id":event.tenant_id,"environment":event.environment,"payment_intent_id":event.payment_intent_id,"attempt_id":event.payment_attempt_id,"reconciliation_case_id":event.reconciliation_case_id,"status":status,"transaction_id":transaction_id}),
        &event.correlation_id,
    )
    .await?;
    tx.commit().await?;
    Ok(GatewayEventDisposition::Applied)
}
