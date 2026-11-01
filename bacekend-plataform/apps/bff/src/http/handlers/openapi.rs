use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        super::health_live::handle,
        super::catalog::plans::create_plan,
        super::catalog::plans::list_plans,
        super::catalog::prices::publish_price,
        super::catalog::capabilities::set_capabilities,
        super::catalog::assignments::assign_plan,
        super::catalog::entitlements::get_entitlement,
        super::catalog::entitlements::list_entitlements,
        super::health_ready::handle,
        super::identity::registration::register,
        super::identity::sessions::login,
        super::identity::sessions::tenant_session,
        super::identity::operator::logout,
        super::identity::operator::operator_login,
        super::identity::operator::operator_session,
        super::identity::operator::operator_logout,
        super::identity::api_keys::create_api_key,
        super::identity::api_keys::list_api_keys,
        super::identity::api_keys::revoke_api_key,
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
