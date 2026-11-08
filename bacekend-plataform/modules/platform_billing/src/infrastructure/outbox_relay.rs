use async_nats::jetstream::Context;
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;

use super::outbox::{self, PendingBillingEvent};

const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn relay_batch(
    pool: &PgPool,
    jetstream: &Context,
    subject_prefix: &str,
) -> Result<usize, sqlx::Error> {
    let events = outbox::claim(pool, 32).await?;
    let mut published = 0;
    for event in events {
        match publish(jetstream, subject_prefix, &event).await {
            Ok(()) => {
                if outbox::acknowledge(pool, event.event_id, event.lease_token).await? {
                    published += 1;
                    tracing::info!(
                        event.name = "outbox.event.published",
                        component = "platform_billing",
                        event_id = %event.event_id,
                        event_type = %event.event_type,
                        correlation_id = %event.correlation_id
                    );
                }
            }
            Err(PublishFailure::Retryable(code)) => {
                let _ = outbox::retry(pool, event.event_id, event.lease_token, code).await?;
                tracing::error!(
                    event.name = "outbox.publish.failed",
                    component = "platform_billing",
                    dependency = "nats_jetstream",
                    error_code = code,
                    error_category = "dependency",
                    retryable = true,
                    event_id = %event.event_id,
                    correlation_id = %event.correlation_id
                );
            }
            Err(PublishFailure::Permanent(code)) => {
                let _ = outbox::quarantine(pool, event.event_id, event.lease_token, code).await?;
                tracing::error!(
                    event.name = "outbox.event.quarantined",
                    component = "platform_billing",
                    error_code = code,
                    error_category = "contract",
                    retryable = false,
                    event_id = %event.event_id,
                    correlation_id = %event.correlation_id
                );
            }
        }
    }
    Ok(published)
}

enum PublishFailure {
    Retryable(&'static str),
    Permanent(&'static str),
}

async fn publish(
    jetstream: &Context,
    subject_prefix: &str,
    event: &PendingBillingEvent,
) -> Result<(), PublishFailure> {
    if subject_prefix.is_empty()
        || !event.event_type.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'.'
                || byte == b'_'
                || byte == b'-'
        })
    {
        return Err(PublishFailure::Permanent("billing_event_subject_invalid"));
    }
    let subject = format!("{subject_prefix}.{}", event.event_type);
    let envelope = json!({
        "event_id": event.event_id,
        "event_type": event.event_type,
        "schema_version": event.schema_version,
        "tenant_id": event.tenant_id,
        "aggregate_id": event.aggregate_id,
        "aggregate_version": event.aggregate_version,
        "audience": event.audience,
        "correlation_id": event.correlation_id,
        "traceparent": event.traceparent,
        "occurred_at": event.occurred_at.to_rfc3339(),
        "payload": event.payload
    });
    let payload = serde_json::to_vec(&envelope)
        .map_err(|_| PublishFailure::Permanent("billing_event_serialization_failed"))?;
    let mut headers = async_nats::HeaderMap::new();
    headers.append("Nats-Msg-Id", event.event_id.to_string());
    headers.append("Payvia-Event-Type", event.event_type.clone());
    let acknowledgement = tokio::time::timeout(
        PUBLISH_TIMEOUT,
        jetstream.publish_with_headers(subject, headers, payload.into()),
    )
    .await
    .map_err(|_| PublishFailure::Retryable("jetstream_publish_timeout"))?
    .map_err(|_| PublishFailure::Retryable("jetstream_publish_unavailable"))?;
    tokio::time::timeout(PUBLISH_TIMEOUT, acknowledgement)
        .await
        .map_err(|_| PublishFailure::Retryable("jetstream_ack_timeout"))?
        .map(|_| ())
        .map_err(|_| PublishFailure::Retryable("jetstream_ack_unavailable"))
}
