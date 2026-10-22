use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(super::health_live::handle, super::health_ready::handle),
    components(schemas(
        crate::http::dto::health_response::HealthResponse,
        crate::http::dto::error_envelope::ErrorEnvelope
    ))
)]
pub struct ApiDocument;

pub async fn handle() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDocument::openapi())
}
