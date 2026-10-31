use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        super::health_live::handle,
        super::catalog::create_plan,
        super::catalog::list_plans,
        super::catalog::publish_price,
        super::catalog::set_capabilities,
        super::catalog::assign_plan,
        super::catalog::get_entitlement,
        super::catalog::list_entitlements,
        super::health_ready::handle,
        super::identity::register,
        super::identity::login,
        super::identity::tenant_session,
        super::identity::logout,
        super::identity::operator_login,
        super::identity::operator_session,
        super::identity::operator_logout,
        super::identity::create_api_key,
        super::identity::list_api_keys,
        super::identity::revoke_api_key,
        super::tenant::register,
        super::tenant::get,
        super::tenant::list_for_operator,
        super::tenant::change_status
    ),
    components(schemas(
        crate::http::dto::health_response::HealthResponse,
        crate::http::dto::error_envelope::ErrorEnvelope,
        super::identity::Credentials,
        super::identity::RegisteredUser,
        super::identity::SessionStarted,
        super::identity::TenantSessionResponse,
        super::identity::OperatorSessionResponse,
        super::identity::NewApiKey,
        super::identity::ApiKeyCreated,
        super::identity::ApiKeyMetadataResponse,
        super::tenant::NewTenant,
        super::tenant::ChangeTenantStatus,
        super::tenant::TenantView,
        super::tenant::TenantList,
        super::catalog::NewPlan,
        super::catalog::NewPriceVersion,
        super::catalog::CapabilityDefinition,
        super::catalog::SetCapabilities,
        super::catalog::AssignPlan,
        super::catalog::PlanView,
        super::catalog::PriceView,
        super::catalog::EntitlementView
    ))
)]
pub struct ApiDocument;

pub async fn handle() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDocument::openapi())
}
