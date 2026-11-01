use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
pub struct EventEnvelope {
    pub event_id: Uuid,
    pub event_type: String,
    pub schema_version: i16,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub audience: String,
    pub correlation_id: String,
    pub traceparent: Option<String>,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}
