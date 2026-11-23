use opentelemetry::propagation::Injector;
use std::time::Duration;
use tonic::{
    Request, Status,
    metadata::{MetadataKey, MetadataMap, MetadataValue},
};

pub(super) fn contextual_request<T>(body: T, correlation_id: &str) -> Result<Request<T>, Status> {
    let mut request = Request::new(body);
    let value = MetadataValue::try_from(correlation_id)
        .map_err(|_| Status::invalid_argument("correlation identifier is invalid"))?;
    request.metadata_mut().insert("x-correlation-id", value);
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject(&mut GrpcMetadataInjector(request.metadata_mut()))
    });
    request.set_timeout(Duration::from_secs(5));
    Ok(request)
}

struct GrpcMetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for GrpcMetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        let Ok(key) = MetadataKey::from_bytes(key.as_bytes()) else {
            tracing::warn!(
                event.name = "grpc.trace_context.inject.failed",
                component = "entitlements_quotas",
                operation = "dependency_request",
                error_code = "trace_context_metadata_invalid"
            );
            return;
        };
        let Ok(value) = MetadataValue::try_from(value) else {
            tracing::warn!(
                event.name = "grpc.trace_context.inject.failed",
                component = "entitlements_quotas",
                operation = "dependency_request",
                error_code = "trace_context_metadata_invalid"
            );
            return;
        };
        self.0.insert(key, value);
    }
}
