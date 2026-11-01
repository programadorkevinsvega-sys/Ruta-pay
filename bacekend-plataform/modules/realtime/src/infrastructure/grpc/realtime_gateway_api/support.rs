use std::time::Duration;

use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
use platform_contracts::realtime::RealtimeEvent;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tonic::{Request, Status};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::infrastructure::persistence::{self, JournalError};

pub(super) async fn send_replay(
    pool: &PgPool,
    audience: &str,
    tenant_id: Option<Uuid>,
    cursor: &mut i64,
    sender: &mpsc::Sender<Result<RealtimeEvent, Status>>,
    correlation_id: &str,
) -> Result<(), ()> {
    loop {
        let started = std::time::Instant::now();
        let mut attempt = 0_u32;
        let events = loop {
            match persistence::replay(pool, audience, tenant_id, *cursor, BATCH_SIZE).await {
                Ok(events) => break events,
                Err(error @ JournalError::Database(_)) if attempt < 2 => {
                    attempt += 1;
                    tracing::warn!(event.name = "realtime.journal.replay.retry", component = "realtime", operation = "replay_journal", dependency = "postgresql", retryable = true, attempt, correlation_id = %correlation_id);
                    tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
                    drop(error);
                }
                Err(error) => {
                    let error_code = match &error {
                        JournalError::InvalidEnvelope => "realtime_scope_invalid".to_owned(),
                        JournalError::Database(database_error) => database_error
                            .as_database_error()
                            .and_then(|cause| cause.code())
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "database_error".to_owned()),
                        JournalError::Redis => "realtime_signal_unavailable".to_owned(),
                        JournalError::InvalidPersistedEvent => "realtime_event_corrupt".to_owned(),
                        JournalError::CursorOutOfRange => "realtime_cursor_out_of_range".to_owned(),
                        JournalError::CursorGap => "realtime_cursor_gap".to_owned(),
                    };
                    let retryable =
                        matches!(&error, JournalError::Database(_) | JournalError::Redis);
                    tracing::error!(event.name = "realtime.journal.replay.failed", component = "realtime", operation = "replay_journal", dependency = "postgresql", error_code, error_category = "persistence", retryable, dependency_duration_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id);
                    let _ = sender.send(Err(map_journal_error(error))).await;
                    return Err(());
                }
            }
        };
        if events.is_empty() {
            return Ok(());
        }
        let has_more = events.len() == BATCH_SIZE as usize;
        for event in events {
            *cursor = event.cursor;
            sender.send(Ok(to_proto(event))).await.map_err(|_| ())?;
        }
        if !has_more {
            return Ok(());
        }
    }
}

pub(super) fn parse_scope(audience: &str, tenant_id: &str) -> Result<Option<Uuid>, Status> {
    match audience {
        "platform" if tenant_id.is_empty() => Ok(None),
        "tenant" => Uuid::parse_str(tenant_id)
            .map(Some)
            .map_err(|_| Status::invalid_argument("tenant scope is invalid")),
        _ => Err(Status::invalid_argument("realtime audience is invalid")),
    }
}

pub(super) fn to_proto(event: persistence::JournalEvent) -> RealtimeEvent {
    RealtimeEvent {
        cursor: event.cursor as u64,
        event_id: event.event_id.to_string(),
        event_type: event.event_type,
        aggregate_id: event.aggregate_id.to_string(),
        aggregate_version: event.aggregate_version as u64,
        audience: event.audience,
        tenant_id: event
            .tenant_id
            .map_or_else(String::new, |tenant_id| tenant_id.to_string()),
        correlation_id: event.correlation_id,
        payload_json: event.payload.to_string(),
    }
}

pub(super) fn map_journal_error(error: JournalError) -> Status {
    match error {
        JournalError::InvalidEnvelope => Status::invalid_argument("realtime request is invalid"),
        JournalError::Database(_) => Status::internal("realtime journal operation failed"),
        JournalError::Redis => Status::unavailable("realtime signal is temporarily unavailable"),
        JournalError::InvalidPersistedEvent => {
            Status::internal("realtime journal operation failed")
        }
        JournalError::CursorOutOfRange => {
            Status::out_of_range("realtime cursor is outside the retained scope")
        }
        JournalError::CursorGap => {
            Status::failed_precondition("realtime journal contains a cursor gap; restart replay")
        }
    }
}

pub(super) fn request_span<T>(
    request: &Request<T>,
    operation: &'static str,
    correlation_id: &str,
) -> tracing::Span {
    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&MetadataExtractor(request.metadata()))
    });
    let trace_id = parent.span().span_context().trace_id().to_string();
    let span = tracing::info_span!("grpc.module.realtime", operation, correlation_id = %correlation_id, trace_id = %trace_id);
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(event.name = "realtime.trace.parent.failed", component = "realtime", operation, error_code = "trace_parent_rejected", error_category = "observability", correlation_id = %correlation_id, error_class = %error);
    }
    span
}

struct MetadataExtractor<'a>(&'a tonic::metadata::MetadataMap);
impl opentelemetry::propagation::Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|key| match key {
                tonic::metadata::KeyRef::Ascii(key) => Some(key.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}
