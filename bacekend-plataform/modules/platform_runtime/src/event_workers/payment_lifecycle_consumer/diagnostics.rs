use async_nats::jetstream;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use opentelemetry::trace::TraceContextExt;
use payvia_observability::{extract_trace_context, new_correlation_id, valid_correlation_id};
use platform_payments::application::payment_service::PaymentServiceError;
use serde_json::Value;
use std::{env, time::Duration};
use tokio::sync::watch;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub(super) fn message_context(payload: &[u8]) -> (Option<String>, String) {
    let event = serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|value| value.get("event").cloned());
    let traceparent = event
        .as_ref()
        .and_then(|event| event.get("traceparent"))
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 128)
        .map(str::to_owned);
    let correlation_id = event
        .as_ref()
        .and_then(|event| event.get("correlation_id"))
        .and_then(Value::as_str)
        .filter(|value| valid_correlation_id(value))
        .map(str::to_owned)
        .unwrap_or_else(new_correlation_id);
    (traceparent, correlation_id)
}

pub(super) fn consumer_span(
    name: &'static str,
    correlation_id: &str,
    traceparent: Option<&str>,
) -> tracing::Span {
    let span = tracing::info_span!(
        "payments.gateway_event.consume",
        component = "payments",
        operation = name,
        correlation_id = %correlation_id
    );
    let parent = extract_trace_context(traceparent);
    if traceparent.is_some() && !parent.span().span_context().is_valid() {
        tracing::warn!(
            event.name = "payments.gateway_event.trace_context.rejected",
            component = "payments",
            operation = name,
            error_code = "invalid_traceparent",
            correlation_id = %correlation_id
        );
    }
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(
            event.name = "payments.gateway_event.trace_context.rejected",
            component = "payments",
            operation = name,
            error_code = %error,
            correlation_id = %correlation_id
        );
    }
    span
}

pub(super) async fn acknowledge(message: jetstream::Message) {
    if let Err(error) = message.ack().await {
        tracing::error!(
            event.name = "payments.gateway_event.ack.failed",
            component = "payments",
            operation = "acknowledge_gateway_event",
            dependency = "nats_jetstream",
            error_code = "gateway_event_ack_failed",
            error_category = "connectivity",
            retryable = true,
            error_class = %error
        );
    }
}

pub(super) async fn reject_or_retry(message: jetstream::Message, error: PaymentServiceError) {
    let permanent = matches!(
        error,
        PaymentServiceError::ProviderEventInvalid | PaymentServiceError::Serialization(_)
    );
    tracing::error!(
        event.name = "payments.gateway_event.processing.failed",
        component = "payments",
        operation = "apply_gateway_event",
        error_category = if permanent {
            "validation"
        } else {
            "dependency"
        },
        error_code = if permanent {
            "gateway_event_invalid"
        } else {
            "gateway_event_processing_failed"
        },
        retryable = !permanent
    );
    let acknowledgement = if permanent {
        message.ack_with(jetstream::AckKind::Term).await
    } else {
        message
            .ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5))))
            .await
    };
    if let Err(error) = acknowledgement {
        tracing::error!(
            event.name = "payments.gateway_event.negative_ack.failed",
            component = "payments",
            operation = "negative_ack_gateway_event",
            dependency = "nats_jetstream",
            error_code = "gateway_event_negative_ack_failed",
            error_category = "connectivity",
            retryable = true,
            error_class = %error
        );
    }
}

pub(super) fn load_verification_key() -> Result<Vec<u8>, &'static str> {
    let encoded = env::var("PLATFORM_GATEWAY_EVENT_VERIFY_KEY")
        .map_err(|_| "gateway_event_verify_key_missing")?;
    let key = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "gateway_event_verify_key_invalid")?;
    if key.len() < 32 {
        return Err("gateway_event_verify_key_invalid");
    }
    Ok(key)
}

pub(super) async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}
