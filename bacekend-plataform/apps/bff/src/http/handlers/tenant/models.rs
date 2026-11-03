use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct RealtimeQuery {
    pub(super) audience: String,
    pub(super) tenant_id: Option<String>,
    pub(super) cursor: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct RealtimeEventView {
    pub(super) cursor: u64,
    pub(super) event_id: String,
    pub(super) event_type: String,
    pub(super) aggregate_id: String,
    pub(super) aggregate_version: u64,
    pub(super) tenant_id: String,
    pub(super) correlation_id: String,
    pub(super) payload_json: String,
}

#[derive(Serialize)]
pub(super) struct RealtimeReplayView {
    pub(super) events: Vec<RealtimeEventView>,
    pub(super) next_cursor: u64,
    pub(super) has_more: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct NewTenant {
    pub display_name: String,
    pub slug: String,
    pub legal_identification_type: String,
    pub legal_identification_number: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ChangeTenantStatus {
    pub status: String,
    pub reason: String,
    pub expected_version: i64,
}

#[derive(Deserialize)]
pub struct TenantQuery {
    pub query: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[derive(Serialize, ToSchema)]
pub struct TenantView {
    pub tenant_id: String,
    pub display_name: String,
    pub slug: String,
    pub status: String,
    pub status_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct TenantList {
    pub tenants: Vec<TenantView>,
    pub total: i64,
}
