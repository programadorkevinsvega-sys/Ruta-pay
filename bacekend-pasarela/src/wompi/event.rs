use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::WompiEnvironment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WompiPaymentStatus {
    Processing,
    Succeeded,
    Failed,
}

impl WompiPaymentStatus {
    pub fn as_platform_status(self) -> &'static str {
        match self {
            Self::Processing => "processing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, WompiEventError> {
        match value {
            "PENDING" => Ok(Self::Processing),
            "APPROVED" => Ok(Self::Succeeded),
            "DECLINED" | "VOIDED" | "ERROR" => Ok(Self::Failed),
            _ => Err(WompiEventError::UnsupportedStatus),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedWompiEvent {
    pub attempt_id: Uuid,
    pub provider_transaction_reference: String,
    pub provider_event_id: String,
    pub event_id: Uuid,
    pub status: WompiPaymentStatus,
    pub amount_minor: i64,
    pub currency: String,
    pub occurred_at: DateTime<Utc>,
    pub payload_sha256: [u8; 32],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WompiEventError {
    #[error("callback envelope is invalid")]
    InvalidEnvelope,
    #[error("callback signature is invalid")]
    InvalidSignature,
    #[error("callback environment does not match the configured account")]
    EnvironmentMismatch,
    #[error("callback event type is not supported")]
    UnsupportedEvent,
    #[error("callback transaction status is not supported")]
    UnsupportedStatus,
    #[error("callback transaction reference is invalid")]
    InvalidReference,
    #[error("callback transaction amount or currency is invalid")]
    InvalidMoney,
    #[error("callback timestamp is invalid")]
    InvalidTimestamp,
}

pub fn verify_event(
    raw_body: &[u8],
    header_checksum: Option<&str>,
    event_secret: &str,
    expected_environment: WompiEnvironment,
) -> Result<VerifiedWompiEvent, WompiEventError> {
    if raw_body.is_empty() || raw_body.len() > 65_536 || event_secret.trim().is_empty() {
        return Err(WompiEventError::InvalidEnvelope);
    }
    let payload: Value =
        serde_json::from_slice(raw_body).map_err(|_| WompiEventError::InvalidEnvelope)?;
    let signature = payload
        .get("signature")
        .and_then(Value::as_object)
        .ok_or(WompiEventError::InvalidEnvelope)?;
    let properties = signature
        .get("properties")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= 32)
        .ok_or(WompiEventError::InvalidEnvelope)?;
    let body_checksum = signature
        .get("checksum")
        .and_then(Value::as_str)
        .ok_or(WompiEventError::InvalidEnvelope)?;
    let timestamp = payload
        .get("timestamp")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or(WompiEventError::InvalidTimestamp)?;
    let event_name = payload
        .get("event")
        .and_then(Value::as_str)
        .ok_or(WompiEventError::InvalidEnvelope)?;
    if event_name != "transaction.updated" {
        return Err(WompiEventError::UnsupportedEvent);
    }
    let environment = payload
        .get("environment")
        .and_then(Value::as_str)
        .ok_or(WompiEventError::InvalidEnvelope)?;
    if environment != expected_environment.wompi_value() {
        return Err(WompiEventError::EnvironmentMismatch);
    }
    if let Some(header) = header_checksum
        && !constant_time_hex_eq(header, body_checksum)
    {
        return Err(WompiEventError::InvalidSignature);
    }

    let mut signature_input = Vec::with_capacity(256);
    let data = payload
        .get("data")
        .ok_or(WompiEventError::InvalidEnvelope)?;
    for property in properties {
        let path = property
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or(WompiEventError::InvalidEnvelope)?;
        let value = property_value(data, path).ok_or(WompiEventError::InvalidEnvelope)?;
        let scalar = scalar_as_string(value).ok_or(WompiEventError::InvalidEnvelope)?;
        signature_input.extend_from_slice(scalar.as_bytes());
    }
    signature_input.extend_from_slice(timestamp.to_string().as_bytes());
    signature_input.extend_from_slice(event_secret.as_bytes());

    let computed = sha256_hex_upper(&signature_input);
    if !constant_time_hex_eq(&computed, body_checksum) {
        return Err(WompiEventError::InvalidSignature);
    }

    let transaction = data
        .get("transaction")
        .and_then(Value::as_object)
        .ok_or(WompiEventError::InvalidEnvelope)?;
    let transaction_id = required_string(transaction, "id")?;
    if transaction_id.len() > 160
        || !transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(WompiEventError::InvalidEnvelope);
    }
    let reference = required_string(transaction, "reference")?;
    let attempt_id = attempt_id_from_reference(reference)?;
    let status = WompiPaymentStatus::parse(required_string(transaction, "status")?)?;
    let amount_minor = transaction
        .get("amount_in_cents")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or(WompiEventError::InvalidMoney)?;
    let currency = required_string(transaction, "currency")?;
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(WompiEventError::InvalidMoney);
    }

    let occurred_at_millis = if timestamp >= 1_000_000_000_000 {
        timestamp
    } else {
        timestamp
            .checked_mul(1_000)
            .ok_or(WompiEventError::InvalidTimestamp)?
    };
    let occurred_at = DateTime::<Utc>::from_timestamp_millis(occurred_at_millis)
        .ok_or(WompiEventError::InvalidTimestamp)?;
    let payload_sha256: [u8; 32] = Sha256::digest(raw_body).into();
    let provider_event_id = stable_provider_event_id(
        transaction_id,
        event_name,
        required_string(transaction, "status")?,
        timestamp,
    );
    let event_id = stable_uuid(&provider_event_id);

    Ok(VerifiedWompiEvent {
        attempt_id,
        provider_transaction_reference: transaction_id.to_owned(),
        provider_event_id,
        event_id,
        status,
        amount_minor,
        currency: currency.to_owned(),
        occurred_at,
        payload_sha256,
    })
}

pub fn reference_for_attempt(attempt_id: Uuid) -> String {
    format!("rpay_{attempt_id}")
}

fn attempt_id_from_reference(reference: &str) -> Result<Uuid, WompiEventError> {
    let raw = reference
        .strip_prefix("rpay_")
        .ok_or(WompiEventError::InvalidReference)?;
    Uuid::parse_str(raw).map_err(|_| WompiEventError::InvalidReference)
}

fn property_value<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut value = root;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        value = value.get(segment)?;
    }
    Some(value)
}

fn scalar_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, WompiEventError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(WompiEventError::InvalidEnvelope)
}

fn stable_provider_event_id(
    transaction_id: &str,
    event_name: &str,
    status: &str,
    timestamp: i64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(transaction_id.as_bytes());
    digest.update([0]);
    digest.update(event_name.as_bytes());
    digest.update([0]);
    digest.update(status.as_bytes());
    digest.update(timestamp.to_be_bytes());
    hex_upper(&digest.finalize())
}

fn stable_uuid(value: &str) -> Uuid {
    let digest = Sha256::digest(value.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn sha256_hex_upper(value: &[u8]) -> String {
    hex_upper(&Sha256::digest(value))
}

fn hex_upper(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn constant_time_hex_eq(left: &str, right: &str) -> bool {
    if left.len() != 64 || right.len() != 64 {
        return false;
    }
    let left = left.to_ascii_uppercase();
    let right = right.to_ascii_uppercase();
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
