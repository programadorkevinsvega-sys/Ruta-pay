use axum::{
    body::Body,
    extract::MatchedPath,
    http::{HeaderMap, HeaderName, HeaderValue, Request},
    middleware::Next,
    response::Response,
};
use payvia_observability::{
    current_span_id, current_trace_id, new_correlation_id, valid_correlation_id,
};
use payvia_platform_core::diagnostics::extract_remote_context;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

struct InFlightRequest;

impl Drop for InFlightRequest {
    fn drop(&mut self) {
        metrics::gauge!("payvia_http_requests_in_flight").decrement(1.0);
    }
}

pub async fn handle(mut request: Request<Body>, next: Next) -> Response {
    let headers: HeaderMap = request.headers().clone();
    let correlation = headers
        .get("x-correlation-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_correlation_id(value))
        .map(str::to_owned)
        .unwrap_or_else(new_correlation_id);
    let parent = extract_remote_context(&headers);
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched_path| matched_path.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());
    let span = tracing::info_span!("http.server.request", http.request.method = %method, http.route = route, correlation_id = %correlation);
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(event.name = "http.trace_context.rejected", component = "platform_bff", operation = "request", error_class = "trace_context", error_code = %error, correlation_id = %correlation);
    }
    request.headers_mut().insert(
        HeaderName::from_static("x-correlation-id"),
        HeaderValue::from_str(&correlation).unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    async move {
        let started = std::time::Instant::now();
        let trace_id = current_trace_id();
        let span_id = current_span_id();
        metrics::gauge!("payvia_http_requests_in_flight").increment(1.0);
        let in_flight_request = InFlightRequest;
        tracing::info!(event.name = "http.request.started", component = "platform_bff", operation = route, correlation_id = %correlation, trace_id = %trace_id, span_id = %span_id);
        let mut response = next.run(request).await;
        let code = response.headers_mut().remove("x-internal-error-code").and_then(|value| value.to_str().ok().map(str::to_owned));
        let category = response.headers_mut().remove("x-internal-error-category").and_then(|value| value.to_str().ok().map(str::to_owned));
        let status = response.status();
        let duration = started.elapsed();
        let trace_id = current_trace_id();
        let span_id = current_span_id();
        metrics::counter!("payvia_http_requests_total", "method" => method.as_str().to_owned(), "status" => status.as_u16().to_string()).increment(1);
        metrics::histogram!("payvia_http_request_duration_seconds", "method" => method.as_str().to_owned()).record(duration.as_secs_f64());
        tracing::info!(event.name = "http.request.completed", component = "platform_bff", operation = "http", http.status_code = status.as_u16(), duration_total_ms = duration.as_millis() as u64, error_code = code.as_deref().unwrap_or("none"), error_category = category.as_deref().unwrap_or("none"), correlation_id = %correlation, trace_id = %trace_id, span_id = %span_id);
        response.headers_mut().insert("x-correlation-id", HeaderValue::from_str(&correlation).unwrap_or_else(|_| HeaderValue::from_static("invalid")));
        drop(in_flight_request);
        response
    }.instrument(span).await
}
