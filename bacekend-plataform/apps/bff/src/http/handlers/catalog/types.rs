use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct NewPlan {
    pub(super) slug: String,
    pub(super) name: String,
    pub(super) description: String,
}

#[derive(Deserialize, ToSchema)]
pub struct NewPriceVersion {
    pub(super) plan_id: String,
    pub(super) currency: String,
    pub(super) amount_minor: i64,
    pub(super) period_unit: String,
    pub(super) interval_count: u32,
    pub(super) effective_from: String,
    pub(super) effective_until: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CapabilityDefinition {
    pub(super) capability_key: String,
    pub(super) enabled: bool,
    pub(super) limit_value: Option<i64>,
    pub(super) unit: String,
    pub(super) overage_policy: String,
    pub(super) environment: String,
    pub(super) quota_period: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SetCapabilities {
    pub(super) price_version_id: String,
    pub(super) capabilities: Vec<CapabilityDefinition>,
}

#[derive(Deserialize, ToSchema)]
pub struct AssignPlan {
    pub(super) tenant_id: String,
    pub(super) price_version_id: String,
    pub(super) effective_from: String,
    pub(super) effective_until: Option<String>,
}

#[derive(Deserialize)]
pub struct EntitlementQuery {
    pub(super) environment: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PlanView {
    pub(super) plan_id: String,
    pub(super) slug: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) status: String,
    pub(super) created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PriceView {
    pub(super) price_version_id: String,
    pub(super) plan_id: String,
    pub(super) version: u32,
    pub(super) currency: String,
    pub(super) amount_minor: i64,
    pub(super) period_unit: String,
    pub(super) interval_count: u32,
    pub(super) effective_from: String,
    pub(super) effective_until: Option<String>,
    pub(super) published_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct EntitlementView {
    pub(super) tenant_id: String,
    pub(super) price_version_id: String,
    pub(super) capability_key: String,
    pub(super) enabled: bool,
    pub(super) limit_value: Option<i64>,
    pub(super) consumed: i64,
    pub(super) remaining: Option<i64>,
    pub(super) allowed: bool,
    pub(super) reason: String,
    pub(super) environment: String,
}
