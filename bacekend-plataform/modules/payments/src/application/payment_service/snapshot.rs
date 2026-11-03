use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::payment_intent::{PaymentIntent, PaymentMethodType};

use super::PaymentServiceError;

pub(super) fn encode(intent: &PaymentIntent) -> Value {
    json!({
        "payment_intent_id": intent.id.to_string(),
        "tenant_id": intent.tenant_id.to_string(),
        "environment": intent.environment,
        "amount_minor": intent.amount_minor,
        "currency": intent.currency,
        "payment_method_type": intent.payment_method_type.as_str(),
        "merchant_reference": intent.merchant_reference,
        "status": intent.status,
        "created_at": intent.created_at.to_rfc3339()
    })
}

pub(super) fn decode(snapshot: Value) -> Result<PaymentIntent, PaymentServiceError> {
    let id = parse_uuid(&snapshot, "payment_intent_id")?;
    let tenant_id = parse_uuid(&snapshot, "tenant_id")?;
    let environment = text(&snapshot, "environment")?;
    let amount_minor = snapshot["amount_minor"]
        .as_i64()
        .ok_or(PaymentServiceError::SnapshotUnavailable)?;
    let currency = text(&snapshot, "currency")?;
    let payment_method_type = PaymentMethodType::parse(text(&snapshot, "payment_method_type")?)
        .map_err(|_| PaymentServiceError::SnapshotUnavailable)?;
    let merchant_reference = snapshot["merchant_reference"].as_str().map(str::to_owned);
    let status = text(&snapshot, "status")?;
    let created_at = DateTime::parse_from_rfc3339(text(&snapshot, "created_at")?)
        .map_err(|_| PaymentServiceError::SnapshotUnavailable)?
        .with_timezone(&Utc);
    Ok(PaymentIntent {
        id,
        tenant_id,
        environment: environment.to_owned(),
        amount_minor,
        currency: currency.to_owned(),
        payment_method_type,
        merchant_reference,
        status: status.to_owned(),
        created_at,
    })
}

fn parse_uuid(snapshot: &Value, field: &str) -> Result<Uuid, PaymentServiceError> {
    Uuid::parse_str(text(snapshot, field)?).map_err(|_| PaymentServiceError::SnapshotUnavailable)
}

fn text<'a>(snapshot: &'a Value, field: &str) -> Result<&'a str, PaymentServiceError> {
    snapshot[field]
        .as_str()
        .ok_or(PaymentServiceError::SnapshotUnavailable)
}
