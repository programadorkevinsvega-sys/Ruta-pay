use async_nats::jetstream;
use chrono::{DateTime, Utc};
use platform_billing::{
    application::{BillingError, BillingService, RecordUsage},
    infrastructure::inbox::{self, InboxDisposition},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;
use tokio::{sync::watch, time};
use tokio_stream::StreamExt;
use uuid::Uuid;

const EVENT_TYPE: &str = "billing.usage.recorded";
const PRODUCER: &str = "entitlements_quotas";

struct UsageEvent {
    event_id: Uuid,
    tenant_id: Uuid,
    usage_id: Uuid,
    meter_key: String,
    quantity: i64,
    unit: String,
    occurred_at: DateTime<Utc>,
    price_version_id: Uuid,
    environment: String,
    period_start: String,
    period_end: String,
    correlation_id: String,
    envelope: Value,
}

pub(super) async fn run_billing_usage_consumer(
    pool: PgPool,
    consumer: jetstream::consumer::PullConsumer,
    mut shutdown: watch::Receiver<bool>,
) {
    let service = BillingService::new(pool.clone());
    loop {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(
                    event.name = "jetstream.consumer.fetch.failed",
                    component = "platform_billing",
                    operation = "consume_billable_usage",
                    dependency = "nats_jetstream",
                    error_code = "billing_usage_consumer_fetch_failed",
                    error_category = "connectivity",
                    retryable = true,
                    error_class = %error
                );
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        loop {
            tokio::select! {
                result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                message = messages.next() => match message {
                    Some(Ok(message)) => {
                        match parse_event(&message.payload) {
                            Ok(event) => {
                                let inbox_id = match inbox::receive(
                                    &pool, event.tenant_id, PRODUCER, event.event_id,
                                    EVENT_TYPE, 1, &event.envelope,
                                ).await {
                                    Ok(InboxDisposition::Inserted(id)) => Some(id),
                                    Ok(InboxDisposition::Duplicate { id, processed: true }) => {
                                        if let Err(error) = message.ack().await {
                                            tracing::warn!(event.name="jetstream.consumer.ack.failed", component="platform_billing", operation="ack_processed_usage_duplicate", error_class=%error);
                                        }
                                        continue;
                                    }
                                    Ok(InboxDisposition::Duplicate { id, processed: false }) => Some(id),
                                    Err(sqlx::Error::Protocol(_)) => {
                                        term_message(&message).await;
                                        continue;
                                    }
                                    Err(error) => {
                                        retry_message(&message, &event.correlation_id, &error).await;
                                        continue;
                                    }
                                };
                                if let Some(id) = inbox_id {
                                    match persist_usage(&service, &pool, &event, id).await {
                                        Ok(()) => {
                                            if let Err(error) = message.ack().await {
                                                tracing::warn!(event.name="jetstream.consumer.ack.failed", component="platform_billing", operation="ack_billable_usage", correlation_id=%event.correlation_id, error_class=%error);
                                            }
                                        }
                                        Err(UsageFailure::Permanent(code)) => {
                                            let _ = inbox::mark_rejected(&pool, event.tenant_id, id, code).await;
                                            term_message(&message).await;
                                        }
                                        Err(UsageFailure::Retryable(error)) => {
                                            retry_message(&message, &event.correlation_id, &error).await;
                                        }
                                    }
                                }
                            }
                            Err(()) => term_message(&message).await,
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(event.name="jetstream.consumer.stream.failed", component="platform_billing", operation="read_billable_usage", dependency="nats_jetstream", error_code="billing_usage_stream_failed", error_category="connectivity", retryable=true, error_class=%error);
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}

enum UsageFailure {
    Permanent(&'static str),
    Retryable(sqlx::Error),
}

async fn persist_usage(
    service: &BillingService,
    pool: &PgPool,
    event: &UsageEvent,
    inbox_id: Uuid,
) -> Result<(), UsageFailure> {
    let attributes = json!({
        "price_version_id": event.price_version_id,
        "environment": event.environment,
        "period_start": event.period_start,
        "period_end": event.period_end
    });
    service
        .record_usage(RecordUsage {
            tenant_id: event.tenant_id,
            meter_key: &event.meter_key,
            source_system: PRODUCER,
            source_event_id: &event.usage_id.to_string(),
            quantity: event.quantity,
            unit: &event.unit,
            occurred_at: event.occurred_at,
            attributes,
        })
        .await
        .map_err(|error| match error {
            BillingError::Database(database) => UsageFailure::Retryable(database),
            BillingError::IdempotencyConflict => {
                UsageFailure::Permanent("usage_idempotency_conflict")
            }
            BillingError::InvalidCommand => UsageFailure::Permanent("billable_usage_invalid"),
            _ => UsageFailure::Permanent("billable_usage_rejected"),
        })?;
    inbox::mark_processed(pool, event.tenant_id, inbox_id)
        .await
        .map_err(UsageFailure::Retryable)?;
    Ok(())
}

fn parse_event(bytes: &[u8]) -> Result<UsageEvent, ()> {
    let envelope: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    if envelope.get("event_type").and_then(Value::as_str) != Some(EVENT_TYPE)
        || envelope.get("schema_version").and_then(Value::as_i64) != Some(1)
    {
        return Err(());
    }
    let payload = envelope
        .get("payload")
        .filter(|value| value.is_object())
        .ok_or(())?;
    let string = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).ok_or(());
    let event_id = Uuid::parse_str(string(&envelope, "event_id")?).map_err(|_| ())?;
    let tenant_id = Uuid::parse_str(string(payload, "tenant_id")?).map_err(|_| ())?;
    let usage_id = Uuid::parse_str(string(payload, "usage_id")?).map_err(|_| ())?;
    let price_version_id = Uuid::parse_str(string(payload, "price_version_id")?).map_err(|_| ())?;
    let quantity = payload
        .get("quantity")
        .and_then(Value::as_i64)
        .filter(|v| *v > 0)
        .ok_or(())?;
    let environment = string(payload, "environment")?.to_owned();
    if !matches!(environment.as_str(), "test" | "live") {
        return Err(());
    }
    let occurred_at = DateTime::parse_from_rfc3339(string(payload, "occurred_at")?)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| ())?;
    let period_start = string(payload, "period_start")?.to_owned();
    let period_end = string(payload, "period_end")?.to_owned();
    let start = DateTime::parse_from_rfc3339(&period_start).map_err(|_| ())?;
    let end = DateTime::parse_from_rfc3339(&period_end).map_err(|_| ())?;
    if end <= start {
        return Err(());
    }
    let meter_key = string(payload, "meter_key")?.to_owned();
    let unit = string(payload, "unit")?.to_owned();
    let correlation_id = string(&envelope, "correlation_id")?.to_owned();
    if meter_key.is_empty()
        || meter_key.len() > 96
        || unit.is_empty()
        || unit.len() > 32
        || correlation_id.is_empty()
        || correlation_id.len() > 128
    {
        return Err(());
    }
    Ok(UsageEvent {
        event_id,
        tenant_id,
        usage_id,
        meter_key,
        quantity,
        unit,
        occurred_at,
        price_version_id,
        environment,
        period_start,
        period_end,
        correlation_id,
        envelope,
    })
}

async fn term_message(message: &jetstream::Message) {
    if let Err(error) = message.ack_with(jetstream::AckKind::Term).await {
        tracing::error!(event.name="jetstream.consumer.term.failed", component="platform_billing", operation="term_invalid_billable_usage", error_class=%error);
    }
}

async fn retry_message(message: &jetstream::Message, correlation_id: &str, error: &sqlx::Error) {
    let code = error.as_database_error().and_then(|cause| cause.code());
    tracing::error!(
        event.name = "jetstream.consumer.processing.failed",
        component = "platform_billing",
        operation = "persist_billable_usage",
        dependency = "postgresql",
        error_code = code
            .as_deref()
            .unwrap_or("billing_usage_persistence_failed"),
        error_category = "persistence",
        retryable = true,
        correlation_id,
    );
    if let Err(ack_error) = message
        .ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5))))
        .await
    {
        tracing::error!(event.name="jetstream.consumer.nak.failed", component="platform_billing", operation="retry_billable_usage", error_class=%ack_error);
    }
}
