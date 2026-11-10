use super::*;

pub(super) fn validate_event(event: &ProviderEvent) -> Result<DateTime<Utc>, PaymentServiceError> {
    if event.schema_version != 1
        || event.event_id.is_nil()
        || event.tenant_id.is_nil()
        || event.payment_attempt_id.is_nil()
        || event.provider_code.is_empty()
        || event.provider_code.len() > 32
        || !event
            .provider_code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        || event.provider_account_reference.is_empty()
        || event.provider_account_reference.len() > 128
        || event.provider_event_id.is_empty()
        || event.provider_event_id.len() > 160
        || event.provider_transaction_reference.is_empty()
        || event.provider_transaction_reference.len() > 160
        || event.amount_minor <= 0
        || event.currency.len() != 3
        || !event.currency.bytes().all(|byte| byte.is_ascii_uppercase())
        || event.correlation_id.is_empty()
        || event.correlation_id.len() > 128
        || !["test", "live"].contains(&event.environment.as_str())
        || ![
            "processing",
            "requires_action",
            "authorized",
            "succeeded",
            "failed",
        ]
        .contains(&event.status.as_str())
    {
        return Err(PaymentServiceError::ProviderEventInvalid);
    }
    decode_sha256(&event.payload_sha256).ok_or(PaymentServiceError::ProviderEventInvalid)?;
    DateTime::from_timestamp_millis(event.occurred_at_unix_millis)
        .ok_or(PaymentServiceError::ProviderEventInvalid)
}

pub(super) async fn insert_delivery(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &ProviderEvent,
    inbox_id: Uuid,
    disposition: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO payments.provider_event_deliveries (delivery_id,tenant_id,environment,inbox_id,event_id,provider_code,provider_account_reference,provider_event_id,producer_identity,disposition) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'gateway-pasarela:wompi',$9)",
    )
    .bind(Uuid::new_v4())
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(inbox_id)
    .bind(event.event_id)
    .bind(&event.provider_code)
    .bind(&event.provider_account_reference)
    .bind(&event.provider_event_id)
    .bind(disposition)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn mark_inbox(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    inbox_id: Uuid,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE payments.provider_event_inbox SET processing_status=$2,processed_at=now() WHERE inbox_id=$1 AND processing_status='received'",
    )
    .bind(inbox_id)
    .bind(status)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn request_reconciliation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &ProviderEvent,
    payment_intent_id: Uuid,
    reason: &str,
) -> Result<(), sqlx::Error> {
    let open_case: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM payments.payment_reconciliation_cases WHERE tenant_id=$1 AND environment=$2 AND payment_attempt_id=$3 AND status IN ('pending','leased','manual_review') FOR UPDATE",
    )
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(event.payment_attempt_id)
    .fetch_optional(&mut **tx)
    .await?;
    let case_id = match open_case {
        Some(case_id) => case_id,
        None => {
            let case_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO payments.payment_reconciliation_cases (id,tenant_id,environment,payment_intent_id,payment_attempt_id,reason,status,attempts) VALUES ($1,$2,$3,$4,$5,$6,'pending',0)",
            )
            .bind(case_id)
            .bind(event.tenant_id)
            .bind(&event.environment)
            .bind(payment_intent_id)
            .bind(event.payment_attempt_id)
            .bind(reason)
            .execute(&mut **tx)
            .await?;
            case_id
        }
    };
    let _ = sqlx::query(
        "UPDATE payments.payment_reconciliation_cases SET status='pending',lease_token=NULL,lease_until=NULL,payment_transaction_id=NULL,next_attempt_at=now(),last_error_code=NULL,updated_at=now() WHERE id=$1 AND status <> 'resolved'",
    )
    .bind(case_id)
    .execute(&mut **tx)
    .await?;
    let version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(aggregate_version),0)+1 FROM payments.outbox_events WHERE aggregate_id=$1",
    )
    .bind(case_id)
    .fetch_one(&mut **tx)
    .await?;
    let payload = json!({
        "tenant_id":event.tenant_id,
        "environment":event.environment,
        "payment_intent_id":payment_intent_id,
        "attempt_id":event.payment_attempt_id,
        "reconciliation_case_id":case_id,
        "provider_code":event.provider_code,
        "provider_account_reference":event.provider_account_reference,
        "provider_transaction_reference":event.provider_transaction_reference,
        "amount_minor":event.amount_minor,
        "currency":event.currency,
        "reason":reason
    });
    insert_outbox(
        tx,
        case_id,
        version,
        "payment.reconciliation.requested",
        payload,
        &event.correlation_id,
    )
    .await
}

pub(super) async fn audit_service_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &ProviderEvent,
    resource_id: Uuid,
    action: &str,
    details: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,actor_service_identity,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,NULL,'gateway-pasarela:wompi',$3,$4,'completed',$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(event.tenant_id)
    .bind(action)
    .bind(resource_id)
    .bind(&event.correlation_id)
    .bind(payvia_observability::current_trace_id_option())
    .bind(details)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn insert_outbox(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    aggregate_id: Uuid,
    version: i64,
    event_type: &str,
    payload: serde_json::Value,
    correlation_id: &str,
) -> Result<(), sqlx::Error> {
    let audience = if event_type == "payment.reconciliation.requested" {
        "platform"
    } else {
        "tenant"
    };
    sqlx::query(
        "INSERT INTO payments.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,$3,$4,1,$5,$6,$7,$8)",
    )
    .bind(Uuid::new_v4())
    .bind(aggregate_id)
    .bind(version)
    .bind(event_type)
    .bind(audience)
    .bind(payload)
    .bind(payvia_observability::current_traceparent())
    .bind(correlation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) fn attempt_target(value: &str) -> Result<PaymentAttemptStatus, PaymentServiceError> {
    match value {
        "processing" => Ok(PaymentAttemptStatus::Processing),
        "requires_action" => Ok(PaymentAttemptStatus::RequiresAction),
        "authorized" => Ok(PaymentAttemptStatus::Authorized),
        "succeeded" => Ok(PaymentAttemptStatus::Succeeded),
        "failed" => Ok(PaymentAttemptStatus::Failed),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
}

pub(super) fn intent_target(
    value: &str,
) -> Result<Option<PaymentIntentStatus>, PaymentServiceError> {
    match value {
        "processing" | "failed" => Ok(None),
        "requires_action" => Ok(Some(PaymentIntentStatus::RequiresAction)),
        "authorized" => Ok(Some(PaymentIntentStatus::Authorized)),
        "succeeded" => Ok(Some(PaymentIntentStatus::Succeeded)),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
}

pub(super) fn transaction_target(
    value: &str,
) -> Result<PaymentTransactionStatus, PaymentServiceError> {
    match value {
        "processing" | "requires_action" => Ok(PaymentTransactionStatus::Pending),
        "authorized" => Ok(PaymentTransactionStatus::Authorized),
        "succeeded" => Ok(PaymentTransactionStatus::Succeeded),
        "failed" => Ok(PaymentTransactionStatus::Failed),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
}

pub(super) fn transition_attempt(
    current: &str,
    target: PaymentAttemptStatus,
) -> Result<TransitionEffect, PaymentServiceError> {
    let current = match current {
        "created" => PaymentAttemptStatus::Created,
        "submitted" => PaymentAttemptStatus::Submitted,
        "processing" => PaymentAttemptStatus::Processing,
        "requires_action" => PaymentAttemptStatus::RequiresAction,
        "authorized" => PaymentAttemptStatus::Authorized,
        "succeeded" => PaymentAttemptStatus::Succeeded,
        "failed" => PaymentAttemptStatus::Failed,
        "outcome_unknown" => PaymentAttemptStatus::OutcomeUnknown,
        _ => return Err(PaymentServiceError::ProviderEventInvalid),
    };
    Ok(current.transition_to(target, TransitionAuthority::ProviderEvent)?)
}

pub(super) fn transition_intent(
    current: &str,
    target: PaymentIntentStatus,
) -> Result<TransitionEffect, PaymentServiceError> {
    let current = match current {
        "created" => PaymentIntentStatus::Created,
        "processing" => PaymentIntentStatus::Processing,
        "requires_action" => PaymentIntentStatus::RequiresAction,
        "authorized" => PaymentIntentStatus::Authorized,
        "succeeded" => PaymentIntentStatus::Succeeded,
        "outcome_unknown" => PaymentIntentStatus::OutcomeUnknown,
        _ => return Err(PaymentServiceError::ProviderEventInvalid),
    };
    Ok(current.transition_to(target, TransitionAuthority::ProviderEvent)?)
}

pub(super) fn transition_transaction(
    current: &str,
    target: PaymentTransactionStatus,
) -> Result<TransitionEffect, PaymentServiceError> {
    let current = match current {
        "pending" => PaymentTransactionStatus::Pending,
        "authorized" => PaymentTransactionStatus::Authorized,
        "succeeded" => PaymentTransactionStatus::Succeeded,
        "failed" => PaymentTransactionStatus::Failed,
        "outcome_unknown" => PaymentTransactionStatus::OutcomeUnknown,
        _ => return Err(PaymentServiceError::ProviderEventInvalid),
    };
    Ok(current.transition_to(target, TransitionAuthority::ProviderEvent)?)
}

pub(super) fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let bytes: Vec<u8> = (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect::<Option<_>>()?;
    bytes.try_into().ok()
}

pub(super) fn attempt_status_text(status: PaymentAttemptStatus) -> &'static str {
    match status {
        PaymentAttemptStatus::Created => "created",
        PaymentAttemptStatus::Submitted => "submitted",
        PaymentAttemptStatus::Processing => "processing",
        PaymentAttemptStatus::RequiresAction => "requires_action",
        PaymentAttemptStatus::Authorized => "authorized",
        PaymentAttemptStatus::Succeeded => "succeeded",
        PaymentAttemptStatus::Failed => "failed",
        PaymentAttemptStatus::OutcomeUnknown => "outcome_unknown",
    }
}

pub(super) fn intent_status_text(status: PaymentIntentStatus) -> &'static str {
    match status {
        PaymentIntentStatus::Created => "created",
        PaymentIntentStatus::Processing => "processing",
        PaymentIntentStatus::RequiresAction => "requires_action",
        PaymentIntentStatus::Authorized => "authorized",
        PaymentIntentStatus::Succeeded => "succeeded",
        PaymentIntentStatus::OutcomeUnknown => "outcome_unknown",
    }
}

pub(super) fn transaction_status_text(status: PaymentTransactionStatus) -> &'static str {
    match status {
        PaymentTransactionStatus::Pending => "pending",
        PaymentTransactionStatus::Authorized => "authorized",
        PaymentTransactionStatus::Succeeded => "succeeded",
        PaymentTransactionStatus::Failed => "failed",
        PaymentTransactionStatus::OutcomeUnknown => "outcome_unknown",
    }
}
