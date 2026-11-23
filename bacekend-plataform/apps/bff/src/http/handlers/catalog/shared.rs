use axum::http::HeaderMap;
use opentelemetry::propagation::Injector;
use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};

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
    tonic_request_with_timeout(body, correlation, std::time::Duration::from_secs(3))
}

pub(super) fn tonic_request_with_timeout<T>(
    body: T,
    correlation: &str,
    timeout: std::time::Duration,
) -> tonic::Request<T> {
    let mut request = tonic::Request::new(body);
    if let Ok(value) = tonic::metadata::MetadataValue::try_from(correlation) {
        request.metadata_mut().insert("x-correlation-id", value);
    }
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject(&mut GrpcMetadataInjector(request.metadata_mut()))
    });
    request.set_timeout(timeout);
    request
}

struct GrpcMetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for GrpcMetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        let key = match MetadataKey::from_bytes(key.as_bytes()) {
            Ok(key) => key,
            Err(_) => {
                tracing::warn!(
                    event.name = "bff.trace_context.inject.failed",
                    component = "platform_bff",
                    operation = "grpc_request",
                    error_code = "trace_context_metadata_invalid"
                );
                return;
            }
        };
        match MetadataValue::try_from(value) {
            Ok(value) => {
                self.0.insert(key, value);
            }
            Err(_) => {
                tracing::warn!(
                    event.name = "bff.trace_context.inject.failed",
                    component = "platform_bff",
                    operation = "grpc_request",
                    error_code = "trace_context_metadata_invalid"
                );
            }
        }
    }
}
