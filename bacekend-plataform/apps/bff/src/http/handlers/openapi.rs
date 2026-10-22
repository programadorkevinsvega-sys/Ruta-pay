use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        super::health_live::handle,
        super::health_ready::handle,
        super::identity::register,
        super::identity::login,
        super::identity::logout,
        super::identity::create_api_key,
        super::identity::list_api_keys,
        super::identity::revoke_api_key
    ),
    components(schemas(
        crate::http::dto::health_response::HealthResponse,
        crate::http::dto::error_envelope::ErrorEnvelope,
        super::identity::Credentials,
        super::identity::RegisteredUser,
        super::identity::SessionStarted,
        super::identity::NewApiKey,
        super::identity::ApiKeyCreated,
        super::identity::ApiKeyMetadataResponse
    ))
)]
pub struct ApiDocument;

pub async fn handle() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDocument::openapi())
}
