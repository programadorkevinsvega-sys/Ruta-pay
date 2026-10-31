use async_nats::jetstream::Context;
use serde_json::json;
use sqlx::PgPool;

use crate::infrastructure::persistence::{
    PendingOutboxEvent, claim_outbox_events, mark_outbox_published, reschedule_outbox_event,
};

pub async fn relay_batch(
    pool: &PgPool,
    jetstream: &Context,
    subject_prefix: &str,
) -> Result<usize, sqlx::Error> {
    let events = claim_outbox_events(pool, 32).await?;
    let mut delivered = 0;
    for event in events {
        match publish_event(jetstream, &event, subject_prefix).await {
            Ok(()) => {
                let marked = mark_outbox_published(pool, event.event_id, event.lease_token).await?;
                if marked {
                    delivered += 1;
                    tracing::info!(event.name = "outbox.event.published", component = "identity_access", operation = "relay_outbox", event_id = %event.event_id, event_type = %event.event_type, correlation_id = %event.correlation_id, traceparent = event.traceparent.as_deref().unwrap_or("unavailable"));
                } else {
                    tracing::warn!(event.name = "outbox.event.lease_lost", component = "identity_access", operation = "mark_outbox_published", event_id = %event.event_id, correlation_id = %event.correlation_id);
                }
            }
            Err(error_code) => {
                reschedule_outbox_event(pool, event.event_id, event.lease_token, error_code)
                    .await?;
                tracing::error!(event.name = "outbox.publish.failed", component = "identity_access", operation = "publish_outbox_event", dependency = "nats_jetstream", error_code, error_category = "dependency", retryable = true, event_id = %event.event_id, event_type = %event.event_type, correlation_id = %event.correlation_id, traceparent = event.traceparent.as_deref().unwrap_or("unavailable"));
            }
        }
    }
    Ok(delivered)
}

async fn publish_event(
    jetstream: &Context,
    event: &PendingOutboxEvent,
    subject_prefix: &str,
) -> Result<(), &'static str> {
    if event.event_type.is_empty()
        || !event.event_type.bytes().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == b'.'
                || character == b'_'
        })
    {
        return Err("event_type_invalid");
    }
    let subject = format!("{subject_prefix}.{}", event.event_type);
    let envelope = json!({
        "event_id": event.event_id,
        "event_type": event.event_type,
        "schema_version": event.schema_version,
        "aggregate_id": event.aggregate_id,
        "aggregate_version": event.aggregate_version,
        "audience": event.audience,
        "correlation_id": event.correlation_id,
        "traceparent": event.traceparent,
        "occurred_at": chrono::Utc::now(),
        "payload": event.payload,
    });
    let payload = serde_json::to_vec(&envelope).map_err(|_| "event_serialization_failed")?;
    let mut headers = async_nats::HeaderMap::new();
    headers.append("Nats-Msg-Id", event.event_id.to_string());
    headers.append("Payvia-Event-Type", event.event_type.clone());
    let acknowledgement = jetstream
        .publish_with_headers(subject, headers, payload.into())
        .await
        .map_err(|_| "jetstream_publish_unavailable")?;
    acknowledgement
        .await
        .map(|_| ())
        .map_err(|_| "jetstream_ack_unavailable")
}
