use std::time::Instant;

use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use payvia_observability::{
    current_trace_id, extract_trace_context, new_correlation_id, start_sentry_transaction,
    valid_correlation_id,
};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::WebhookDiagnostic;

struct InFlightRequest;

impl Drop for InFlightRequest {
    fn drop(&mut self) {
        metrics::gauge!("payvia_http_requests_in_flight", "component" => "gateway").decrement(1.0);
    }
}

pub(super) async fn request_diagnostics(mut request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_owned();
    let route = match (method.as_str(), request.uri().path()) {
        ("GET", "/health/ready") => "GET /health/ready",
        ("POST", path) if path.starts_with("/v1/webhooks/wompi/") => {
            "POST /v1/webhooks/wompi/{account_reference}"
        }
        _ => "unmatched",
    };
    let correlation_id = request
        .headers()
        .get("x-correlation-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_correlation_id(value))
        .map(str::to_owned)
        .unwrap_or_else(new_correlation_id);
    if let Ok(value) = HeaderValue::from_str(&correlation_id) {
        request.headers_mut().insert("x-correlation-id", value);
    }
    let traceparent = request
        .headers()
        .get("traceparent")
        .and_then(|value| value.to_str().ok());
    let span = tracing::info_span!(
        "gateway.http.request",
        service.name = "payvia-gateway",
        http.request.method = %method,
        http.route = route,
        correlation_id = %correlation_id
    );
    if let Err(error) = span.set_parent(extract_trace_context(traceparent)) {
        tracing::warn!(
            event.name = "gateway.http.trace_context.rejected",
            component = "gateway_http",
            operation = "extract_trace_context",
            error_code = %error,
            correlation_id = %correlation_id
        );
    }

    async move {
        let started_at = Instant::now();
        let sentry_transaction = start_sentry_transaction(route, "http.server");
        metrics::gauge!("payvia_http_requests_in_flight", "component" => "gateway").increment(1.0);
        let in_flight = InFlightRequest;
        tracing::info!(
            event.name = "gateway.http.request.started",
            component = "gateway_http",
            operation = "handle_request",
            http.route = route,
            http.request.method = %method,
            correlation_id = %correlation_id,
            trace_id = %current_trace_id()
        );
        let mut response = next.run(request).await;
        let status = response.status().as_u16();
        let duration = started_at.elapsed();
        let diagnostic = response.extensions().get::<WebhookDiagnostic>().copied();
        sentry_transaction
            .set_http_status(status, diagnostic.map(|diagnostic| diagnostic.category));
        metrics::counter!(
            "payvia_http_requests_total",
            "component" => "gateway",
            "route" => route,
            "method" => method.clone(),
            "status" => status.to_string()
        )
        .increment(1);
        metrics::histogram!(
            "payvia_http_request_duration_seconds",
            "component" => "gateway",
            "route" => route,
            "method" => method.clone()
        )
        .record(duration.as_secs_f64());
        if let Some(diagnostic) = diagnostic {
            tracing::error!(
                event.name = "gateway.http.request.failed",
                component = "gateway_http",
                operation = "handle_request",
                error_code = diagnostic.code,
                error_category = diagnostic.category,
                dependency = diagnostic.dependency,
                retryable = diagnostic.retryable,
                http.route = route,
                http.request.method = %method,
                http.response.status_code = status,
                duration_ms = duration.as_millis() as u64,
                correlation_id = %correlation_id,
                trace_id = %current_trace_id()
            );
            tracing::info!(
                event.name = "gateway.http.request.completed",
                component = "gateway_http",
                operation = "handle_request",
                error_code = diagnostic.code,
                error_category = diagnostic.category,
                http.route = route,
                http.request.method = %method,
                http.response.status_code = status,
                result = "failed",
                duration_ms = duration.as_millis() as u64,
                correlation_id = %correlation_id,
                trace_id = %current_trace_id()
            );
        } else {
            tracing::info!(
                event.name = "gateway.http.request.completed",
                component = "gateway_http",
                operation = "handle_request",
                http.route = route,
                http.request.method = %method,
                http.response.status_code = status,
                result = if status >= 500 { "failed" } else { "completed" },
                duration_ms = duration.as_millis() as u64,
                correlation_id = %correlation_id,
                trace_id = %current_trace_id()
            );
        }
        if let Ok(value) = HeaderValue::from_str(&correlation_id) {
            response.headers_mut().insert("x-correlation-id", value);
        }
        drop(in_flight);
        drop(sentry_transaction);
        response
    }
    .instrument(span)
    .await
}
