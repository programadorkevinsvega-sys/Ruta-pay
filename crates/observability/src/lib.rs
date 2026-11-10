use opentelemetry::{
    propagation::{Extractor, TextMapPropagator},
    trace::{SpanId, TraceContextExt, TraceId},
};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub mod telemetry;

/// Returns the active W3C trace identifier, or `unavailable` outside a valid span.
pub fn current_trace_id() -> String {
    current_trace_id_option().unwrap_or_else(|| "unavailable".to_owned())
}

/// Returns the active W3C trace identifier when the current span belongs to a valid trace.
pub fn current_trace_id_option() -> Option<String> {
    let trace_id = tracing::Span::current()
        .context()
        .span()
        .span_context()
        .trace_id();
    (trace_id != TraceId::INVALID).then(|| trace_id.to_string())
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

pub struct SentryTransactionGuard(Option<sentry::Transaction>);

impl SentryTransactionGuard {
    pub fn set_status(&self, status: sentry::protocol::SpanStatus) {
        if let Some(transaction) = &self.0 {
            transaction.set_status(status);
        }
    }

    pub fn set_http_status(&self, status: u16, error_category: Option<&str>) {
        let status = match (status, error_category) {
            (_, Some("authentication")) | (401, _) => sentry::protocol::SpanStatus::Unauthenticated,
            (_, Some("authorization")) | (403, _) => sentry::protocol::SpanStatus::PermissionDenied,
            (_, Some("validation")) | (400, _) => sentry::protocol::SpanStatus::InvalidArgument,
            (_, Some("conflict")) | (409, _) => sentry::protocol::SpanStatus::AlreadyExists,
            (_, Some("dependency")) | (503, _) => sentry::protocol::SpanStatus::Unavailable,
            (429, _) => sentry::protocol::SpanStatus::ResourceExhausted,
            (404, _) => sentry::protocol::SpanStatus::NotFound,
            (408, _) | (504, _) => sentry::protocol::SpanStatus::DeadlineExceeded,
            (500..=599, _) => sentry::protocol::SpanStatus::InternalError,
            (400..=499, _) => sentry::protocol::SpanStatus::UnknownError,
            _ => sentry::protocol::SpanStatus::Ok,
        };
        self.set_status(status);
    }
}

impl Drop for SentryTransactionGuard {
    fn drop(&mut self) {
        if let Some(transaction) = self.0.take() {
            transaction.finish();
        }
    }
}

/// Starts a Sentry performance transaction linked to the active OpenTelemetry trace.
pub fn start_sentry_transaction(name: &str, operation: &str) -> SentryTransactionGuard {
    let span_context = tracing::Span::current()
        .context()
        .span()
        .span_context()
        .clone();
    let transaction = span_context.is_valid().then(|| {
        let trace_id = span_context
            .trace_id()
            .to_string()
            .parse()
            .expect("valid OpenTelemetry trace ID");
        let parent_span_id = span_context
            .span_id()
            .to_string()
            .parse()
            .expect("valid OpenTelemetry span ID");
        sentry::start_transaction(sentry::TransactionContext::new_with_details(
            name,
            operation,
            trace_id,
            None,
            Some(parent_span_id),
        ))
    });
    SentryTransactionGuard(transaction)
}

/// Extracts a validated W3C trace context from a traceparent header or event field.
pub fn extract_trace_context(traceparent: Option<&str>) -> opentelemetry::Context {
    struct TraceparentCarrier<'a>(Option<&'a str>);

    impl Extractor for TraceparentCarrier<'_> {
        fn get(&self, key: &str) -> Option<&str> {
            key.eq_ignore_ascii_case("traceparent")
                .then_some(self.0)
                .flatten()
        }

        fn keys(&self) -> Vec<&str> {
            vec!["traceparent"]
        }
    }

    TraceContextPropagator::new().extract(&TraceparentCarrier(traceparent))
}

#[cfg(test)]
mod tests {
    use super::{
        current_span_id, current_trace_id, current_traceparent, extract_trace_context,
        new_correlation_id, valid_correlation_id,
    };
    use opentelemetry::trace::{TraceContextExt, TraceId};

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

    #[test]
    fn extracts_valid_w3c_traceparent_without_accepting_malformed_context() {
        let context = extract_trace_context(Some(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ));
        assert_eq!(
            context.span().span_context().trace_id(),
            TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("valid trace id")
        );
        assert!(
            !extract_trace_context(Some("invalid"))
                .span()
                .span_context()
                .is_valid()
        );
        assert!(!extract_trace_context(None).span().span_context().is_valid());
    }
}
