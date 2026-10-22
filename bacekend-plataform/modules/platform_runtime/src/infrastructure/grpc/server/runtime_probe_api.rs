use crate::application::use_cases::check_readiness;
use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
use payvia_observability::{current_span_id, current_trace_id};
use platform_contracts::runtime::{
    ReadinessRequest, ReadinessResponse, runtime_probe_server::RuntimeProbe,
};
use tonic::{Request, Response, Status};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Clone, Default)]
pub struct RuntimeProbeApi;

#[tonic::async_trait]
impl RuntimeProbe for RuntimeProbeApi {
    async fn check_readiness(
        &self,
        request: Request<ReadinessRequest>,
    ) -> Result<Response<ReadinessResponse>, Status> {
        let metadata = request.metadata();
        let correlation_id = metadata
            .get("x-correlation-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("missing")
            .to_owned();
        let parent = global::get_text_map_propagator(|propagator| {
            propagator.extract(&MetadataExtractor(metadata))
        });
        let parent_trace_id = parent.span().span_context().trace_id().to_string();
        let span = tracing::info_span!("module.platform_runtime.check_readiness", correlation_id = %correlation_id);
        if let Err(error) = span.set_parent(parent) {
            tracing::warn!(event.name = "grpc.trace_context.rejected", component = "platform_runtime", operation = "check_readiness", error_class = "trace_context", error_code = %error, correlation_id = %correlation_id, trace_id = %parent_trace_id);
        }
        async move {
            let started = std::time::Instant::now();
            let trace_id = current_trace_id();
            tracing::info!(event.name = "grpc.module.request.started", component = "platform_runtime", operation = "check_readiness", correlation_id = %correlation_id, trace_id = %trace_id, span_id = %current_span_id());
            let result = match check_readiness::execute(correlation_id.clone(), trace_id) {
                Ok(result) => result,
                Err(error) => {
                    metrics::counter!("payvia_grpc_requests_total", "method" => "CheckReadiness", "status" => "invalid_argument").increment(1);
                    tracing::warn!(event.name = "grpc.module.request.failed", component = "platform_runtime", operation = "check_readiness", error_code = "invalid_argument", error_category = "validation", retryable = false, duration_total_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id(), error = %error);
                    return Err(Status::invalid_argument("invalid correlation identifier"));
                }
            };
            let response = ReadinessResponse {
                component: result.component.to_owned(),
                status: result.readiness.as_str().to_owned(),
                correlation_id: result.correlation_id,
                trace_id: result.trace_id,
            };
            let duration = started.elapsed();
            metrics::counter!("payvia_grpc_requests_total", "method" => "CheckReadiness", "status" => "ok").increment(1);
            metrics::histogram!("payvia_grpc_request_duration_seconds", "method" => "CheckReadiness").record(duration.as_secs_f64());
            tracing::info!(event.name = "grpc.module.response", component = "platform_runtime", operation = "check_readiness", result = "success", duration_total_ms = duration.as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
            Ok(Response::new(response))
        }.instrument(span).await
    }
}

struct MetadataExtractor<'a>(&'a tonic::metadata::MetadataMap);

impl Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|key| match key {
                tonic::metadata::KeyRef::Ascii(value) => Some(value.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}
