use crate::diagnostics::{
    current_trace_id, extract_remote_context, new_correlation_id, valid_correlation_id,
};
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::{net::TcpListener, sync::watch};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Clone)]
struct MetricsState {
    handle: PrometheusHandle,
    token: Option<Arc<[u8]>>,
}

pub async fn serve(
    listener: TcpListener,
    handle: PrometheusHandle,
    token: Option<String>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), std::io::Error> {
    let router = Router::new()
        .route("/metrics", get(scrape))
        .with_state(MetricsState {
            handle,
            token: token.map(|value| Arc::<[u8]>::from(value.into_bytes())),
        });
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await
}

async fn scrape(State(state): State<MetricsState>, headers: HeaderMap) -> Response {
    let correlation_id = headers
        .get("x-correlation-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_correlation_id(value))
        .map(str::to_owned)
        .unwrap_or_else(new_correlation_id);
    let parent = extract_remote_context(&headers);
    let span = tracing::info_span!("metrics.scrape", correlation_id = %correlation_id);
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(event.name = "metrics.trace_context.rejected", component = "metrics_exporter", operation = "scrape", error_code = %error, correlation_id = %correlation_id);
    }
    async move {
        let mut response = if let Some(expected) = state.token.as_deref() {
            if !authorized(&headers, expected) {
                tracing::warn!(event.name = "metrics.scrape.denied", component = "metrics_exporter", operation = "scrape", error_code = "unauthorized_scrape", error_category = "authorization", retryable = false, correlation_id = %correlation_id, trace_id = %current_trace_id());
                StatusCode::UNAUTHORIZED.into_response()
            } else {
                state.handle.render().into_response()
            }
        } else {
            state.handle.render().into_response()
        };
        if response.status().is_success() {
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            );
        }
        response.headers_mut().insert("x-correlation-id", HeaderValue::from_str(&correlation_id).unwrap_or_else(|_| HeaderValue::from_static("invalid")));
        response
    }.instrument(span).await
}

fn authorized(headers: &HeaderMap, expected: &[u8]) -> bool {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some(actual) = value.strip_prefix("Bearer ") else {
        return false;
    };
    actual.len() == expected.len() && bool::from(actual.as_bytes().ct_eq(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_authorization_requires_exact_token() {
        let mut headers = HeaderMap::new();
        assert!(!authorized(&headers, b"metrics-secret"));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer incorrect"),
        );
        assert!(!authorized(&headers, b"metrics-secret"));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer metrics-secret"),
        );
        assert!(authorized(&headers, b"metrics-secret"));
    }
}
