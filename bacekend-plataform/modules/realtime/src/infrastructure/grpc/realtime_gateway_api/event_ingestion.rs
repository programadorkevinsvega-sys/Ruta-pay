use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::SIGNAL_CHANNEL;
use crate::infrastructure::persistence::{self, JournalError};

pub async fn persist_and_publish(
    pool: &PgPool,
    redis: &mut ConnectionManager,
    envelope: &[u8],
) -> Result<bool, JournalError> {
    let event: persistence::EventEnvelope =
        serde_json::from_slice(envelope).map_err(|_| JournalError::InvalidEnvelope)?;
    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&TraceparentExtractor(event.traceparent.as_deref()))
    });
    let trace_id = parent.span().span_context().trace_id().to_string();
    let span = tracing::info_span!(
        "realtime.persist_and_publish",
        event_id = %event.event_id,
        event_type = %event.event_type,
        correlation_id = %event.correlation_id,
        trace_id = %trace_id
    );
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(
            event.name = "realtime.trace.parent.failed",
            component = "realtime",
            operation = "persist_and_publish",
            error_code = "trace_parent_rejected",
            error_category = "observability",
            correlation_id = %event.correlation_id,
            error_class = %error
        );
    }
    async {
        let inserted = persistence::persist_event(pool, &event).await?;
        redis::cmd("PUBLISH")
            .arg(SIGNAL_CHANNEL)
            .arg("wake")
            .query_async::<i64>(redis)
            .await
            .map_err(|_| JournalError::Redis)?;
        Ok(inserted)
    }
    .instrument(span)
    .await
}

struct TraceparentExtractor<'a>(Option<&'a str>);

impl Extractor for TraceparentExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        if key == "traceparent" { self.0 } else { None }
    }

    fn keys(&self) -> Vec<&str> {
        vec!["traceparent"]
    }
}
