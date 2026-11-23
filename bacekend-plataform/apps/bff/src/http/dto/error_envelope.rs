use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: String,
    pub category: String,
    pub message: &'static str,
    pub correlation_id: String,
    pub trace_id: String,
    pub retryable: bool,
}
