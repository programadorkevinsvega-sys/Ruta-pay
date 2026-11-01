use opentelemetry::trace::{SpanId, TraceContextExt, TraceId};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub mod telemetry;

/// Returns the active W3C trace identifier, or `unavailable` outside a valid span.
pub fn current_trace_id() -> String {
    let trace_id = tracing::Span::current()
        .context()
        .span()
        .span_context()
        .trace_id();
    if trace_id == TraceId::INVALID {
        "unavailable".to_owned()
    } else {
        trace_id.to_string()
    }
}

/// Returns the active span identifier, or `unavailable` outside a valid span.
pub fn current_span_id() -> String {
    let span_id = tracing::Span::current()
        .context()
        .span()
        .span_context()
        .span_id();
    if span_id == SpanId::INVALID {
        "unavailable".to_owned()
    } else {
        span_id.to_string()
    }
}

/// Creates an opaque request correlation identifier suitable for logs and headers.
pub fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Restricts externally supplied correlation identifiers to bounded printable tokens.
pub fn valid_correlation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::{
        current_span_id, current_trace_id, current_traceparent, new_correlation_id,
        valid_correlation_id,
    };

    #[test]
    fn creates_valid_correlation_ids() {
        assert!(valid_correlation_id(&new_correlation_id()));
    }

    #[test]
    fn rejects_empty_oversized_or_injected_correlation_ids() {
        assert!(!valid_correlation_id(""));
        assert!(!valid_correlation_id(&"a".repeat(129)));
        assert!(!valid_correlation_id("corr\nsecret=value"));
        assert!(valid_correlation_id("corr_01-ab.cd"));
    }

    #[test]
    fn trace_identifiers_have_a_safe_fallback_without_a_span() {
        assert_eq!(current_trace_id(), "unavailable");
        assert_eq!(current_span_id(), "unavailable");
    }

    #[test]
    fn traceparent_has_no_fabricated_context_without_a_valid_span() {
        assert!(current_traceparent().is_none());
    }
}

/// Returns the W3C traceparent for the active span, or None outside a valid trace.
pub fn current_traceparent() -> Option<String> {
    let context = tracing::Span::current()
        .context()
        .span()
        .span_context()
        .clone();
    if !context.is_valid() {
        return None;
    }
    Some(format!(
        "00-{}-{}-{:02x}",
        context.trace_id(),
        context.span_id(),
        context.trace_flags().to_u8()
    ))
}
