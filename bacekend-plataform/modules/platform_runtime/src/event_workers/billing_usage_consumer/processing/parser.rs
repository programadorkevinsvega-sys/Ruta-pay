use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::EVENT_TYPE;

struct UsageEvent {
    pub(super) event_id: Uuid,
    pub(super) tenant_id: Uuid,
    pub(super) usage_id: Uuid,
    pub(super) meter_key: String,
    pub(super) quantity: i64,
    pub(super) unit: String,
    pub(super) occurred_at: DateTime<Utc>,
    pub(super) price_version_id: Uuid,
    pub(super) environment: String,
    pub(super) period_start: String,
    pub(super) period_end: String,
    pub(super) correlation_id: String,
    pub(super) envelope: Value,
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ()> {
    value.get(key).and_then(Value::as_str).ok_or(())
}

pub(super) fn parse_event(bytes: &[u8]) -> Result<UsageEvent, ()> {
    let envelope: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    if envelope.get("event_type").and_then(Value::as_str) != Some(EVENT_TYPE)
        || envelope.get("schema_version").and_then(Value::as_i64) != Some(1)
    {
        return Err(());
    }
    let payload = envelope
        .get("payload")
        .filter(|value| value.is_object())
        .ok_or(())?;
    let event_id = Uuid::parse_str(required_string(&envelope, "event_id")?).map_err(|_| ())?;
    let tenant_id = Uuid::parse_str(required_string(payload, "tenant_id")?).map_err(|_| ())?;
    let usage_id = Uuid::parse_str(required_string(payload, "usage_id")?).map_err(|_| ())?;
    let price_version_id =
        Uuid::parse_str(required_string(payload, "price_version_id")?).map_err(|_| ())?;
    let quantity = payload
        .get("quantity")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or(())?;
    let environment = required_string(payload, "environment")?.to_owned();
    if !matches!(environment.as_str(), "test" | "live") {
        return Err(());
    }
    let occurred_at = DateTime::parse_from_rfc3339(required_string(payload, "occurred_at")?)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| ())?;
    let period_start = required_string(payload, "period_start")?.to_owned();
    let period_end = required_string(payload, "period_end")?.to_owned();
    let start = DateTime::parse_from_rfc3339(&period_start).map_err(|_| ())?;
    let end = DateTime::parse_from_rfc3339(&period_end).map_err(|_| ())?;
    if end <= start {
        return Err(());
    }
    let meter_key = required_string(payload, "meter_key")?.to_owned();
    let unit = required_string(payload, "unit")?.to_owned();
    let correlation_id = required_string(&envelope, "correlation_id")?.to_owned();
    if meter_key.is_empty()
        || meter_key.len() > 96
        || unit.is_empty()
        || unit.len() > 32
        || !payvia_observability::valid_correlation_id(&correlation_id)
    {
        return Err(());
    }
    Ok(UsageEvent {
        event_id,
        tenant_id,
        usage_id,
        meter_key,
        quantity,
        unit,
        occurred_at,
        price_version_id,
        environment,
        period_start,
        period_end,
        correlation_id,
        envelope,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_event;
    use chrono::{Duration, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn valid_envelope() -> Vec<u8> {
        let now = Utc::now();
        serde_json::to_vec(&json!({
            "event_id": Uuid::new_v4(),
            "event_type": "billing.usage.recorded",
            "schema_version": 1,
            "correlation_id": "billing-test-001",
            "payload": {
                "tenant_id": Uuid::new_v4(),
                "usage_id": Uuid::new_v4(),
                "price_version_id": Uuid::new_v4(),
                "meter_key": "api.requests",
                "quantity": 1,
                "unit": "request",
                "occurred_at": now.to_rfc3339(),
                "environment": "test",
                "period_start": (now - Duration::days(1)).to_rfc3339(),
                "period_end": (now + Duration::days(1)).to_rfc3339()
            }
        }))
        .expect("test event serializes")
    }

    #[test]
    fn accepts_versioned_usage_event_with_safe_correlation_id() {
        let bytes = valid_envelope();
        let event = parse_event(&bytes).expect("valid usage event is accepted");
        assert_eq!(event.quantity, 1);
        assert_eq!(event.meter_key, "api.requests");
    }

    #[test]
    fn rejects_invalid_correlation_id_and_non_positive_quantity() {
        let mut value: serde_json::Value =
            serde_json::from_slice(&valid_envelope()).expect("valid fixture parses");
        value["correlation_id"] = json!("billing\nforged");
        assert!(parse_event(&serde_json::to_vec(&value).expect("fixture serializes")).is_err());

        let mut value: serde_json::Value =
            serde_json::from_slice(&valid_envelope()).expect("valid fixture parses");
        value["payload"]["quantity"] = json!(0);
        assert!(parse_event(&serde_json::to_vec(&value).expect("fixture serializes")).is_err());
    }

    #[test]
    fn rejects_unknown_schema_versions_and_invalid_periods() {
        let mut value: serde_json::Value =
            serde_json::from_slice(&valid_envelope()).expect("valid fixture parses");
        value["schema_version"] = json!(2);
        assert!(parse_event(&serde_json::to_vec(&value).expect("fixture serializes")).is_err());

        let mut value: serde_json::Value =
            serde_json::from_slice(&valid_envelope()).expect("valid fixture parses");
        value["payload"]["period_end"] = value["payload"]["period_start"].clone();
        assert!(parse_event(&serde_json::to_vec(&value).expect("fixture serializes")).is_err());
    }
}
