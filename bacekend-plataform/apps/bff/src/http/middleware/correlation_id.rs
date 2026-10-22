use axum::http::HeaderMap;
use payvia_observability::{new_correlation_id, valid_correlation_id};

pub fn correlation_id(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_correlation_id(value))
        .map(str::to_owned)
        .unwrap_or_else(new_correlation_id)
}
