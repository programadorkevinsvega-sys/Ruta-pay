use crate::{
    common::grpc::runtime_probe_client::RuntimeProbeClient,
    http::{
        dto::{
            error_envelope::{ErrorBody, ErrorEnvelope},
            health_response::HealthResponse,
        },
        middleware::correlation_id::correlation_id,
        routes::AppState,
    },
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use payvia_observability::{current_span_id, current_trace_id};
use std::{sync::Arc, time::Duration};
use tracing::Instrument;

#[utoipa::path(get, path = "/health/ready", responses((status = 200, body = HealthResponse), (status = 503, body = ErrorEnvelope)))]
pub async fn handle(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let correlation_id = correlation_id(&headers);
    let span = tracing::info_span!("bff.runtime.readiness", correlation_id = %correlation_id);
    async move {
        let trace_id = current_trace_id();
        tracing::info!(event.name = "bff.dependency.call.started", component = "platform_bff", operation = "health_ready", dependency = "platform_runtime", correlation_id = %correlation_id, trace_id = %trace_id, span_id = %current_span_id());
        let dependency_started = std::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(2), RuntimeProbeClient::check(&state.grpc_client, &correlation_id)).await;
        match result {
            Ok(Ok(response)) if response.status == "ready" && response.correlation_id == correlation_id && response.trace_id == current_trace_id() => {
                let dependency_duration = dependency_started.elapsed();
                metrics::counter!("payvia_dependency_requests_total", "dependency" => "platform_runtime", "result" => "success").increment(1);
                metrics::histogram!("payvia_dependency_request_duration_seconds", "dependency" => "platform_runtime", "operation" => "health_ready").record(dependency_duration.as_secs_f64());
                metrics::gauge!("payvia_dependency_up", "dependency" => "platform_runtime").set(1.0);
                tracing::info!(event.name = "bff.dependency.call.completed", component = "platform_bff", operation = "health_ready", dependency = "platform_runtime", result = "success", dependency_duration_ms = dependency_duration.as_millis() as u64, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                (StatusCode::OK, Json(HealthResponse { status: "ready", correlation_id, trace_id: current_trace_id() })).into_response()
            }
            Ok(Ok(_)) => failure("module_contract_invalid", "contract", false, StatusCode::BAD_GATEWAY, dependency_started.elapsed().as_millis() as u64, correlation_id),
            Ok(Err(status)) => {
                let (code, category, retryable, status_code) = match status.code() {
                    tonic::Code::Unavailable => ("module_unavailable", "dependency", true, StatusCode::SERVICE_UNAVAILABLE),
                    tonic::Code::DeadlineExceeded => ("module_timeout", "timeout", true, StatusCode::SERVICE_UNAVAILABLE),
                    tonic::Code::InvalidArgument => ("module_contract_invalid", "contract", false, StatusCode::BAD_GATEWAY),
                    tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => ("module_rejected", "authorization", false, StatusCode::BAD_GATEWAY),
                    _ => ("module_failure", "internal", false, StatusCode::BAD_GATEWAY),
                };
                let dependency_duration_ms = dependency_started.elapsed().as_millis() as u64;
                tracing::warn!(event.name = "bff.dependency.call.failed", component = "platform_bff", operation = "health_ready", dependency = "platform_runtime", error_code = %status.code(), error_category = category, retryable, dependency_duration_ms, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                failure(code, category, retryable, status_code, dependency_duration_ms, correlation_id)
            }
            Err(_) => {
                let dependency_duration_ms = dependency_started.elapsed().as_millis() as u64;
                tracing::warn!(event.name = "bff.dependency.call.failed", component = "platform_bff", operation = "health_ready", dependency = "platform_runtime", error_code = "deadline_exceeded", error_category = "timeout", retryable = true, dependency_duration_ms, correlation_id = %correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                failure("module_timeout", "timeout", true, StatusCode::SERVICE_UNAVAILABLE, dependency_duration_ms, correlation_id)
            }
        }
    }.instrument(span).await
}

fn failure(
    code: &'static str,
    category: &'static str,
    retryable: bool,
    status: StatusCode,
    dependency_duration_ms: u64,
    correlation_id: String,
) -> Response {
    let trace_id = current_trace_id();
    metrics::counter!("payvia_dependency_requests_total", "dependency" => "platform_runtime", "result" => "failure").increment(1);
    metrics::histogram!("payvia_dependency_request_duration_seconds", "dependency" => "platform_runtime", "operation" => "health_ready").record(dependency_duration_ms as f64 / 1000.0);
    metrics::gauge!("payvia_dependency_up", "dependency" => "platform_runtime").set(0.0);
    tracing::error!(event.name = "bff.request.failed", component = "platform_bff", operation = "health_ready", error_code = code, error_category = category, retryable, dependency_duration_ms, correlation_id = %correlation_id, trace_id = %trace_id, span_id = %current_span_id());
    let body = ErrorEnvelope {
        error: ErrorBody {
            code,
            category,
            message: "The readiness check could not be completed.",
            correlation_id,
            trace_id,
            retryable,
        },
    };
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert("x-internal-error-code", HeaderValue::from_static(code));
    response.headers_mut().insert(
        "x-internal-error-category",
        HeaderValue::from_static(category),
    );
    response
}
