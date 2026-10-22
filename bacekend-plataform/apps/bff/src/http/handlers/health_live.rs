use crate::http::{
    dto::health_response::HealthResponse, middleware::correlation_id::correlation_id,
};
use axum::{Json, http::HeaderMap};
use payvia_observability::current_trace_id;

#[utoipa::path(get, path = "/health/live", responses((status = 200, body = HealthResponse)))]
pub async fn handle(headers: HeaderMap) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "alive",
        correlation_id: correlation_id(&headers),
        trace_id: current_trace_id(),
    })
}
