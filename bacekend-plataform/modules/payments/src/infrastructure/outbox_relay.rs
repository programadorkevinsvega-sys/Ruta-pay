use std::time::Duration;

use async_nats::jetstream::Context;
use serde_json::json;
use sqlx::PgPool;

use crate::infrastructure::persistence::{
    PendingOutboxEvent, claim_outbox_events, extend_outbox_lease, mark_outbox_failed,
    mark_outbox_published, reschedule_outbox_event,
};

const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);

enum PublishFailure {
    Retryable(&'static str),
    Permanent(&'static str),
}

pub async fn relay_batch(
    pool: &PgPool,
    jetstream: &Context,
    subject_prefix: &str,
) -> Result<usize, sqlx::Error> {
    let events = claim_outbox_events(pool, 32).await?;
    let mut delivered = 0;
    for event in events {
        if !extend_outbox_lease(pool, event.event_id, event.lease_token).await? {
            tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="relay_outbox",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
            continue;
        }
        match publish_event(jetstream, &event, subject_prefix).await {
            Ok(()) => {
                if mark_outbox_published(pool, event.event_id, event.lease_token).await? {
                    delivered += 1;
                    tracing::info!(event.name="outbox.event.published",component="payments",operation="relay_outbox",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                } else {
                    tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="acknowledge_outbox_event",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                }
            }
            Err(PublishFailure::Retryable(error_code)) => {
                if reschedule_outbox_event(pool, event.event_id, event.lease_token, error_code)
                    .await?
                {
                    tracing::error!(event.name="outbox.publish.failed",component="payments",operation="publish_outbox_event",dependency="nats_jetstream",error_code,error_category="dependency",retryable=true,event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                } else {
                    tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="reschedule_outbox_event",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                }
            }
            Err(PublishFailure::Permanent(error_code)) => {
                if mark_outbox_failed(pool, event.event_id, event.lease_token, error_code).await? {
                    tracing::error!(event.name="outbox.event.quarantined",component="payments",operation="quarantine_outbox_event",error_code,error_category="contract",retryable=false,event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                } else {
                    tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="quarantine_outbox_event",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                }
            }
        }
    }
    Ok(delivered)
}

async fn publish_event(
    jetstream: &Context,
    event: &PendingOutboxEvent,
    subject_prefix: &str,
) -> Result<(), PublishFailure> {
    if event.event_type.is_empty()
        || !event.event_type.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'_'
        })
    {
        return Err(PublishFailure::Permanent("event_type_invalid"));
    }
    let subject = format!("{subject_prefix}.{}", event.event_type);
    let audience = if event.audience == "platform" {
        "platform_operator"
    } else {
        event.audience.as_str()
    };
    let payload = serde_json::to_vec(&json!({
        "event_id": event.event_id,
        "event_type": event.event_type,
        "schema_version": event.schema_version,
        "aggregate_id": event.aggregate_id,
        "aggregate_version": event.aggregate_version,
        "audience": audience,
        "correlation_id": event.correlation_id,
        "traceparent": event.traceparent,
        "occurred_at": event.occurred_at.to_rfc3339(),
        "payload": event.payload
    }))
    .map_err(|_| PublishFailure::Permanent("event_serialization_failed"))?;
    let mut headers = async_nats::HeaderMap::new();
    headers.append("Nats-Msg-Id", event.event_id.to_string());
    headers.append("Payvia-Event-Type", event.event_type.clone());
    let ack = tokio::time::timeout(
        PUBLISH_TIMEOUT,
        jetstream.publish_with_headers(subject, headers, payload.into()),
    )
    .await
    .map_err(|_| PublishFailure::Retryable("jetstream_publish_timeout"))?
    .map_err(|_| PublishFailure::Retryable("jetstream_publish_unavailable"))?;
    tokio::time::timeout(PUBLISH_TIMEOUT, ack)
        .await
        .map_err(|_| PublishFailure::Retryable("jetstream_ack_timeout"))?
        .map(|_| ())
        .map_err(|_| PublishFailure::Retryable("jetstream_ack_unavailable"))
}
