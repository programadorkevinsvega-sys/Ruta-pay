use std::time::Instant;

use tonic::Status;

use super::mapping::error_details;

pub(super) fn dependency_error(
    status: Status,
    operation: &'static str,
    dependency: &'static str,
    correlation_id: &str,
    duration_ms: u64,
) -> Status {
    let (error_code, error_category) = error_details(&status);
    tracing::warn!(
        event.name = "grpc.dependency.call.failed",
        component = "entitlements_quotas",
        operation,
        dependency,
        grpc_code = ?status.code(),
        error_code,
        error_category,
        retryable = false,
        dependency_duration_ms = duration_ms,
        correlation_id,
        trace_id = %payvia_observability::current_trace_id(),
        span_id = %payvia_observability::current_span_id()
    );
    status
}

pub(super) fn record_module_result(
    operation: &'static str,
    correlation_id: &str,
    started: Instant,
    result: &Result<impl Sized, Status>,
) {
    let duration = started.elapsed();
    let duration_ms = duration.as_millis() as u64;
    metrics::histogram!("payvia_grpc_request_duration_seconds", "method" => operation)
        .record(duration.as_secs_f64());
    match result {
        Ok(_) => {
            metrics::counter!("payvia_grpc_requests_total", "method" => operation, "status" => "ok")
                .increment(1);
            tracing::info!(
                event.name = "grpc.module.response",
                component = "entitlements_quotas",
                operation,
                result = "success",
                duration_total_ms = duration_ms,
                correlation_id,
                trace_id = %payvia_observability::current_trace_id(),
                span_id = %payvia_observability::current_span_id()
            );
        }
        Err(status) => {
            let (error_code, error_category) = error_details(status);
            let status_code = format!("{:?}", status.code());
            metrics::counter!("payvia_grpc_requests_total", "method" => operation, "status" => status_code)
                .increment(1);
            tracing::warn!(
                event.name = "grpc.module.request.failed",
                component = "entitlements_quotas",
                operation,
                dependency = "entitlements_quotas",
                grpc_code = ?status.code(),
                error_code,
                error_category,
                retryable = false,
                duration_total_ms = duration_ms,
                correlation_id,
                trace_id = %payvia_observability::current_trace_id(),
                span_id = %payvia_observability::current_span_id()
            );
        }
    }
}
