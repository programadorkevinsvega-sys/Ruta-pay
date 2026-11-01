use axum::http::HeaderMap;

use crate::http::handlers::identity;

const OPERATOR_SESSION: &str = "payvia_platform_session";
const OPERATOR_CSRF: &str = "payvia_platform_csrf";

pub(super) fn operator_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    if !identity::valid_origin(headers) || !identity::valid_csrf(headers, OPERATOR_CSRF) {
        return None;
    }
    Some((
        identity::cookie_value(headers, OPERATOR_SESSION)?,
        identity::cookie_value(headers, OPERATOR_CSRF)?,
    ))
}

pub(super) fn tonic_request<T>(body: T, correlation: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(body);
    if let Ok(value) = tonic::metadata::MetadataValue::try_from(correlation) {
        request.metadata_mut().insert("x-correlation-id", value);
    }
    request.set_timeout(std::time::Duration::from_secs(3));
    request
}
