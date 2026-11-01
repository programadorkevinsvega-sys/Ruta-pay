use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub struct NewPlan {
    slug: String,
    name: String,
    description: String,
}

#[derive(Deserialize, ToSchema)]
pub struct NewPriceVersion {
    plan_id: String,
    currency: String,
    amount_minor: i64,
    period_unit: String,
    interval_count: u32,
    effective_from: String,
    effective_until: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CapabilityDefinition {
    capability_key: String,
    enabled: bool,
    limit_value: Option<i64>,
    unit: String,
    overage_policy: String,
    environment: String,
    quota_period: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SetCapabilities {
    price_version_id: String,
    capabilities: Vec<CapabilityDefinition>,
}

#[derive(Deserialize, ToSchema)]
pub struct AssignPlan {
    tenant_id: String,
    price_version_id: String,
    effective_from: String,
    effective_until: Option<String>,
}

#[derive(Deserialize)]
pub struct EntitlementQuery {
    environment: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PlanView {
    plan_id: String,
    slug: String,
    name: String,
    description: String,
    status: String,
    created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PriceView {
    price_version_id: String,
    plan_id: String,
    version: u32,
    currency: String,
    amount_minor: i64,
    period_unit: String,
    interval_count: u32,
    effective_from: String,
    effective_until: Option<String>,
    published_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct EntitlementView {
    tenant_id: String,
    price_version_id: String,
    capability_key: String,
    enabled: bool,
    limit_value: Option<i64>,
    consumed: i64,
    remaining: Option<i64>,
    allowed: bool,
    reason: String,
    environment: String,
}
