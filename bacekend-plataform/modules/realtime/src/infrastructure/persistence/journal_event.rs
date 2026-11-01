use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub struct JournalEvent {
    pub cursor: i64,
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub audience: String,
    pub tenant_id: Option<Uuid>,
    pub correlation_id: String,
    pub payload: Value,
}
