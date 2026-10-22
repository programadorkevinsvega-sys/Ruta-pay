use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        super::health_live::handle,
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
        super::tenant::TenantList
    ))
)]
pub struct ApiDocument;

pub async fn handle() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDocument::openapi())
}
