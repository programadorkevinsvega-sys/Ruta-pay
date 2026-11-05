use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ReconciliationResult {
    pub(super) schema_version: u32,
    pub(super) event_id: Uuid,
    pub(super) event_type: String,
    pub(super) provider_code: String,
    pub(super) provider_account_reference: String,
    pub(super) tenant_id: Uuid,
    pub(super) environment: String,
    pub(super) payment_intent_id: Uuid,
    pub(super) payment_attempt_id: Uuid,
    pub(super) reconciliation_case_id: Uuid,
    pub(super) provider_transaction_reference: Option<String>,
    pub(super) outcome: String,
    pub(super) status: Option<String>,
    pub(super) amount_minor: Option<i64>,
    pub(super) currency: Option<String>,
    pub(super) evidence_sha256: Option<String>,
    pub(super) safe_error_code: Option<String>,
    pub(super) correlation_id: String,
    pub(super) occurred_at_unix_millis: i64,
}

pub(super) async fn record_manual_review(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &ReconciliationResult,
) -> Result<bool, sqlx::Error> {
    let next_attempt_number: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(attempt_number),0)+1 FROM payments.payment_reconciliation_attempts WHERE tenant_id=$1 AND environment=$2 AND reconciliation_case_id=$3",
    )
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(event.reconciliation_case_id)
    .fetch_one(&mut **tx)
    .await?;
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO payments.payment_reconciliation_attempts (id,tenant_id,environment,reconciliation_case_id,attempt_number,result_status,provider_transaction_reference,safe_error_code,queried_at,result_event_id) VALUES ($1,$2,$3,$4,$5,'inconclusive',$6,$7,now(),$8) ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(event.tenant_id)
    .bind(&event.environment)
    .bind(event.reconciliation_case_id)
    .bind(next_attempt_number)
    .bind(&event.provider_transaction_reference)
    .bind(event.safe_error_code.as_deref().unwrap_or("provider_lookup_inconclusive"))
    .bind(event.event_id)
    .fetch_optional(&mut **tx)
    .await?;
    if inserted.is_none() {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE payments.payment_reconciliation_cases SET status='manual_review',attempts=attempts+1,lease_token=NULL,lease_until=NULL,last_error_code=$2,updated_at=now() WHERE id=$1 AND status <> 'resolved'",
    )
    .bind(event.reconciliation_case_id)
    .bind(event.safe_error_code.as_deref().unwrap_or("provider_lookup_inconclusive"))
    .execute(&mut **tx)
    .await?;
    audit_result(
        tx,
        event,
        event.payment_attempt_id,
        "payment_reconciliation.manual_review",
        json!({"case_id":event.reconciliation_case_id,"reason":event.safe_error_code}),
    )
    .await?;
    Ok(true)
}

pub(super) fn validate_result(
    event: &ReconciliationResult,
) -> Result<DateTime<Utc>, PaymentServiceError> {
    if event.schema_version != 1
        || event.event_id.is_nil()
        || event.event_type != "payment.provider_reconciliation.result"
        || event.provider_code != "wompi"
        || event.provider_account_reference.is_empty()
        || event.tenant_id.is_nil()
        || event.payment_intent_id.is_nil()
        || event.payment_attempt_id.is_nil()
        || event.reconciliation_case_id.is_nil()
        || !["test", "live"].contains(&event.environment.as_str())
        || event.correlation_id.is_empty()
        || event.correlation_id.len() > 128
        || !["resolved", "manual_review"].contains(&event.outcome.as_str())
    {
        return Err(PaymentServiceError::ProviderEventInvalid);
    }
    if event.outcome == "resolved"
        && !matches!(event.status.as_deref(), Some("succeeded" | "failed"))
    {
        return Err(PaymentServiceError::ProviderEventInvalid);
    }
    if let Some(code) = &event.safe_error_code {
        if code.is_empty()
            || code.len() > 64
            || !code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(PaymentServiceError::ProviderEventInvalid);
        }
    }
    DateTime::<Utc>::from_timestamp_millis(event.occurred_at_unix_millis)
        .ok_or(PaymentServiceError::ProviderEventInvalid)
}

pub(super) async fn audit_result(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &ReconciliationResult,
    resource_id: Uuid,
    action: &str,
    details: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO payments.audit_events (event_id,tenant_id,actor_key_id,actor_service_identity,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,NULL,'gateway-pasarela:reconciliation',$3,$4,'completed',$5,NULL,$6)",
    )
    .bind(Uuid::new_v4())
    .bind(event.tenant_id)
    .bind(action)
    .bind(resource_id)
    .bind(&event.correlation_id)
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
    sqlx::query(
        "INSERT INTO payments.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,$3,$4,1,'tenant',$5,$6)",
    )
    .bind(Uuid::new_v4())
    .bind(aggregate_id)
    .bind(version.max(1))
    .bind(event_type)
    .bind(payload)
    .bind(correlation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) fn parse_attempt_target(
    value: &str,
) -> Result<PaymentAttemptStatus, PaymentServiceError> {
    match value {
        "succeeded" => Ok(PaymentAttemptStatus::Succeeded),
        "failed" => Ok(PaymentAttemptStatus::Failed),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
}

pub(super) fn parse_attempt_status(
    value: &str,
) -> Result<PaymentAttemptStatus, PaymentServiceError> {
    match value {
        "created" => Ok(PaymentAttemptStatus::Created),
        "submitted" => Ok(PaymentAttemptStatus::Submitted),
        "processing" => Ok(PaymentAttemptStatus::Processing),
        "requires_action" => Ok(PaymentAttemptStatus::RequiresAction),
        "authorized" => Ok(PaymentAttemptStatus::Authorized),
        "succeeded" => Ok(PaymentAttemptStatus::Succeeded),
        "failed" => Ok(PaymentAttemptStatus::Failed),
        "outcome_unknown" => Ok(PaymentAttemptStatus::OutcomeUnknown),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
}

pub(super) fn parse_intent_status(value: &str) -> Result<PaymentIntentStatus, PaymentServiceError> {
    match value {
        "created" => Ok(PaymentIntentStatus::Created),
        "processing" => Ok(PaymentIntentStatus::Processing),
        "requires_action" => Ok(PaymentIntentStatus::RequiresAction),
        "authorized" => Ok(PaymentIntentStatus::Authorized),
        "succeeded" => Ok(PaymentIntentStatus::Succeeded),
        "outcome_unknown" => Ok(PaymentIntentStatus::OutcomeUnknown),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
}

pub(super) fn parse_transaction_target(
    value: &str,
) -> Result<PaymentTransactionStatus, PaymentServiceError> {
    match value {
        "succeeded" => Ok(PaymentTransactionStatus::Succeeded),
        "failed" => Ok(PaymentTransactionStatus::Failed),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
}

pub(super) fn parse_transaction_status(
    value: &str,
) -> Result<PaymentTransactionStatus, PaymentServiceError> {
    match value {
        "pending" => Ok(PaymentTransactionStatus::Pending),
        "authorized" => Ok(PaymentTransactionStatus::Authorized),
        "succeeded" => Ok(PaymentTransactionStatus::Succeeded),
        "failed" => Ok(PaymentTransactionStatus::Failed),
        "outcome_unknown" => Ok(PaymentTransactionStatus::OutcomeUnknown),
        _ => Err(PaymentServiceError::ProviderEventInvalid),
    }
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

pub(super) async fn record_conflict(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &mut ReconciliationResult,
    error_code: &str,
) -> Result<GatewayEventDisposition, PaymentServiceError> {
    event.safe_error_code = Some(error_code.to_owned());
    let recorded = record_manual_review(tx, event).await?;
    Ok(if recorded {
        GatewayEventDisposition::ManualReview
    } else {
        GatewayEventDisposition::Duplicate
    })
}
