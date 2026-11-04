use serde_json::json;
use sqlx::{PgRow, Row};
use uuid::Uuid;

use crate::domain::{
    payment_lifecycle::PaymentAttemptStatus,
    payment_intent::{PaymentIntent, PaymentIntentError, PaymentMethodType},
    payment_lifecycle::{
        PaymentEnvironment, PaymentIntentStatus, TransitionAuthority, TransitionEffect,
    },
};

use super::{
    PaymentIntentStatusUpdate, PaymentService, PaymentServiceError,
    UpdatePaymentIntentStatusCommand,
};

pub(super) async fn update(
    service: &PaymentService,
    command: UpdatePaymentIntentStatusCommand,
) -> Result<PaymentIntentStatusUpdate, PaymentServiceError> {
    PaymentEnvironment::parse(&command.environment)?;
    if command.expected_version <= 0
        || command.correlation_id.is_empty()
        || command.correlation_id.len() > 128
        || command.correlation_id.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(crate::domain::payment_lifecycle::LifecycleModelError::IdentifierInvalid.into());
    }
    let (next_intent, next_attempt, authority) = match command.status.as_str() {
        "processing" => (
            PaymentIntentStatus::Processing,
            PaymentAttemptStatus::Submitted,
            TransitionAuthority::OperationSubmitted,
        ),
        "outcome_unknown" => (
            PaymentIntentStatus::OutcomeUnknown,
            PaymentAttemptStatus::OutcomeUnknown,
            TransitionAuthority::OperationTimeout,
        ),
        _ => return Err(PaymentServiceError::StatusUpdateNotAllowed),
    };

    let mut tx = service.pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(command.tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let intent_row = sqlx::query(
        "SELECT id,tenant_id,environment,amount_minor,currency,payment_method_type,merchant_reference,status,aggregate_version,created_at FROM payments.payment_intents WHERE id=$1 AND tenant_id=$2 AND environment=$3 FOR UPDATE",
    )
    .bind(command.payment_intent_id)
    .bind(command.tenant_id)
    .bind(&command.environment)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentIntentError::NotFound)?;
    let current_intent = intent_from_row(&intent_row)?;
    let current_intent_status = parse_intent_status(&current_intent.status)?;
    let intent_version: i64 = intent_row.try_get("aggregate_version")?;

    let attempt_row = sqlx::query(
        "SELECT id,status,aggregate_version FROM payments.payment_attempts WHERE id=$1 AND tenant_id=$2 AND environment=$3 AND payment_intent_id=$4 FOR UPDATE",
    )
    .bind(command.payment_attempt_id)
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(command.payment_intent_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentIntentError::NotFound)?;
    let current_attempt = parse_attempt_status(
        &attempt_row.try_get::<String, _>("status")?,
    )?;
    let attempt_version: i64 = attempt_row.try_get("aggregate_version")?;

    if current_intent_status == next_intent {
        let expected_attempt = next_attempt.as_str();
        if current_attempt.as_str() != expected_attempt {
            return Err(PaymentServiceError::VersionConflict);
        }
        audit_status(
            &mut tx,
            &command,
            "payment_intent.status_transition_replayed",
            json!({
                "status": command.status,
                "aggregate_version": intent_version,
                "attempt_id": command.payment_attempt_id
            }),
        )
        .await?;
        tx.commit().await?;
        return Ok(PaymentIntentStatusUpdate {
            intent: current_intent,
            aggregate_version: intent_version,
        });
    }

    if intent_version != command.expected_version {
        return Err(PaymentServiceError::VersionConflict);
    }
    if next_intent == PaymentIntentStatus::Processing
        && current_intent_status != PaymentIntentStatus::Created
    {
        return Err(PaymentServiceError::StatusUpdateNotAllowed);
    }
    if next_intent == PaymentIntentStatus::OutcomeUnknown
        && current_intent_status != PaymentIntentStatus::Processing
    {
        return Err(PaymentServiceError::StatusUpdateNotAllowed);
    }

    let attempt_effect = current_attempt.transition_to(next_attempt, authority)?;
    let intent_effect = current_intent_status.transition_to(next_intent, authority)?;
    if attempt_effect != TransitionEffect::Apply || intent_effect != TransitionEffect::Apply {
        return Err(PaymentServiceError::VersionConflict);
    }

    let attempt_new_version: i64 = sqlx::query_scalar(
        "UPDATE payments.payment_attempts SET status=$2,aggregate_version=aggregate_version+1,updated_at=now(),last_event_at=now() WHERE id=$1 AND aggregate_version=$3 RETURNING aggregate_version",
    )
    .bind(command.payment_attempt_id)
    .bind(next_attempt.as_str())
    .bind(attempt_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentServiceError::VersionConflict)?;
    let intent_new_version: i64 = sqlx::query_scalar(
        "UPDATE payments.payment_intents SET status=$2,aggregate_version=aggregate_version+1,updated_at=now(),last_event_at=now() WHERE id=$1 AND tenant_id=$3 AND environment=$4 AND aggregate_version=$5 RETURNING aggregate_version",
    )
    .bind(command.payment_intent_id)
    .bind(command.status.as_str())
    .bind(command.tenant_id)
    .bind(&command.environment)
    .bind(command.expected_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PaymentServiceError::VersionConflict)?;

    audit_status(
        &mut tx,
        &command,
        "payment_intent.status_changed",
        json!({
            "from_status": current_intent.status,
            "to_status": command.status,
            "aggregate_version": intent_new_version,
            "attempt_id": command.payment_attempt_id
        }),
    )
    .await?;
    audit_status(
        &mut tx,
        &command,
        "payment_attempt.status_changed",
        json!({
            "to_status": next_attempt.as_str(),
            "aggregate_version": attempt_new_version,
            "attempt_id": command.payment_attempt_id
        }),
    )
    .await?;

    insert_outbox(
        &mut tx,
        command.payment_intent_id,
        intent_new_version,
        "payment_intent.status_changed",
        json!({
            "tenant_id": command.tenant_id,
            "environment": command.environment,
            "payment_intent_id": command.payment_intent_id,
            "attempt_id": command.payment_attempt_id,
            "status": command.status,
            "aggregate_version": intent_new_version
        }),
        &command.correlation_id,
    )
    .await?;
    insert_outbox(
        &mut tx,
        command.payment_attempt_id,
        attempt_new_version,
        "payment_attempt.status_changed",
        json!({
            "tenant_id": command.tenant_id,
            "environment": command.environment,
            "payment_intent_id": command.payment_intent_id,
            "attempt_id": command.payment_attempt_id,
            "status": next_attempt.as_str(),
            "aggregate_version": attempt_new_version
        }),
        &command.correlation_id,
    )
    .await?;

    if next_intent == PaymentIntentStatus::OutcomeUnknown {
        let case_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM payments.payment_reconciliation_cases WHERE tenant_id=$1 AND environment=$2 AND payment_attempt_id=$3 AND status IN ('pending','leased','manual_review') FOR UPDATE",
        )
        .bind(command.tenant_id)
        .bind(&command.environment)
        .bind(command.payment_attempt_id)
        .fetch_optional(&mut *tx)
        .await?;
        if case_id.is_none() {
            let related_transaction: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM payments.payment_transactions WHERE tenant_id=$1 AND environment=$2 AND payment_intent_id=$3 AND payment_attempt_id=$4 ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
            )
            .bind(command.tenant_id)
            .bind(&command.environment)
            .bind(command.payment_intent_id)
            .bind(command.payment_attempt_id)
            .fetch_optional(&mut *tx)
            .await?;
            let case_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO payments.payment_reconciliation_cases (id,tenant_id,environment,payment_intent_id,payment_attempt_id,payment_transaction_id,reason,status,attempts) VALUES ($1,$2,$3,$4,$5,$6,'outcome_unknown','pending',0)",
            )
            .bind(case_id)
            .bind(command.tenant_id)
            .bind(&command.environment)
            .bind(command.payment_intent_id)
            .bind(command.payment_attempt_id)
            .bind(related_transaction)
            .execute(&mut *tx)
            .await?;
            insert_outbox(
                &mut tx,
                case_id,
                1,
                "payment.reconciliation.requested",
                json!({
                    "tenant_id": command.tenant_id,
                    "environment": command.environment,
                    "payment_intent_id": command.payment_intent_id,
                    "attempt_id": command.payment_attempt_id,
                    "reconciliation_case_id": case_id,
                    "reason": "outcome_unknown"
                }),
                &command.correlation_id,
            )
            .await?;
        }
    }

    let mut intent = current_intent;
    intent.status = command.status;
    tx.commit().await?;
    Ok(PaymentIntentStatusUpdate {
        intent,
        aggregate_version: intent_new_version,
    })
}

async fn audit_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    command: &UpdatePaymentIntentStatusCommand,
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
    .bind(command.payment_intent_id)
    .bind(&command.correlation_id)
    .bind(payvia_observability::current_trace_id())
    .bind(details)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_outbox(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    aggregate_id: Uuid,
    aggregate_version: i64,
    event_type: &str,
    payload: serde_json::Value,
    correlation_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO payments.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,$3,$4,1,'tenant',$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(aggregate_id)
    .bind(aggregate_version)
    .bind(event_type)
    .bind(payload)
    .bind(payvia_observability::current_traceparent())
    .bind(correlation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn intent_from_row(row: &PgRow) -> Result<PaymentIntent, PaymentServiceError> {
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

fn parse_intent_status(value: &str) -> Result<PaymentIntentStatus, PaymentServiceError> {
    match value {
        "created" => Ok(PaymentIntentStatus::Created),
        "processing" => Ok(PaymentIntentStatus::Processing),
        "requires_action" => Ok(PaymentIntentStatus::RequiresAction),
        "authorized" => Ok(PaymentIntentStatus::Authorized),
        "succeeded" => Ok(PaymentIntentStatus::Succeeded),
        "outcome_unknown" => Ok(PaymentIntentStatus::OutcomeUnknown),
        _ => Err(PaymentServiceError::SnapshotUnavailable),
    }
}

fn parse_attempt_status(value: &str) -> Result<PaymentAttemptStatus, PaymentServiceError> {
    match value {
        "created" => Ok(PaymentAttemptStatus::Created),
        "submitted" => Ok(PaymentAttemptStatus::Submitted),
        "processing" => Ok(PaymentAttemptStatus::Processing),
        "requires_action" => Ok(PaymentAttemptStatus::RequiresAction),
        "authorized" => Ok(PaymentAttemptStatus::Authorized),
        "succeeded" => Ok(PaymentAttemptStatus::Succeeded),
        "failed" => Ok(PaymentAttemptStatus::Failed),
        "outcome_unknown" => Ok(PaymentAttemptStatus::OutcomeUnknown),
        _ => Err(PaymentServiceError::SnapshotUnavailable),
    }
}

impl PaymentAttemptStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Submitted => "submitted",
            Self::Processing => "processing",
            Self::RequiresAction => "requires_action",
            Self::Authorized => "authorized",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}
