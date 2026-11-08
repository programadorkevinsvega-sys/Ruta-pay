use async_nats::jetstream;
use chrono::{DateTime, Utc};
use metrics::{counter, histogram};
use platform_billing::{
    application::{BillingError, BillingService, RecordUsage},
    infrastructure::inbox::{self, InboxDisposition},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::{Duration, Instant};
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

pub(super) async fn process_message(
    message: jetstream::Message,
    service: &BillingService,
    pool: &PgPool,
) {
    let _timer = UsageProcessingTimer(Instant::now());
    let event = match parse_event(&message.payload) {
        Ok(event) => event,
        Err(()) => {
            counter!("platform_billing_usage_events_total", "outcome" => "invalid").increment(1);
            tracing::warn!(
                event.name = "billing.usage.event.rejected",
                component = "platform_billing",
                operation = "parse_billable_usage",
                error_code = "billing_usage_event_invalid",
                error_category = "contract",
                retryable = false
            );
            term_message(&message, None, None).await;
            return;
        }
    };

    tracing::debug!(
        event.name = "billing.usage.event.received",
        component = "platform_billing",
        operation = "consume_billable_usage",
        event_id = %event.event_id,
        correlation_id = %event.correlation_id
    );

    let inbox_id = match inbox::receive(
        pool,
        event.tenant_id,
        PRODUCER,
        event.event_id,
        EVENT_TYPE,
        1,
        &event.envelope,
    )
    .await
    {
        Ok(InboxDisposition::Inserted(id)) => id,
        Ok(InboxDisposition::Duplicate { processed: true, .. }) => {
            counter!("platform_billing_usage_events_total", "outcome" => "duplicate_processed")
                .increment(1);
            tracing::info!(
                event.name = "billing.usage.event.duplicate",
                component = "platform_billing",
                operation = "ack_processed_usage_duplicate",
                event_id = %event.event_id,
                correlation_id = %event.correlation_id
            );
            ack_message(&message, &event).await;
            return;
        }
        Ok(InboxDisposition::Duplicate { id, processed: false }) => id,
        Err(sqlx::Error::Protocol(_)) => {
            counter!("platform_billing_usage_events_total", "outcome" => "idempotency_conflict")
                .increment(1);
            tracing::error!(
                event.name = "billing.usage.event.conflict",
                component = "platform_billing",
                operation = "receive_billable_usage",
                event_id = %event.event_id,
                correlation_id = %event.correlation_id,
                error_code = "billing_inbox_idempotency_conflict",
                error_category = "integrity",
                retryable = false
            );
            term_message(&message, Some(event.event_id), Some(&event.correlation_id)).await;
            return;
        }
        Err(error) => {
            counter!("platform_billing_usage_events_total", "outcome" => "inbox_retry").increment(1);
            retry_message(&message, &event, &error).await;
            return;
        }
    };

    match persist_usage(service, &event, inbox_id).await {
        Ok(duplicate) => {
            let outcome = if duplicate { "duplicate_usage" } else { "processed" };
            counter!("platform_billing_usage_events_total", "outcome" => outcome).increment(1);
            tracing::info!(
                event.name = "billing.usage.event.processed",
                component = "platform_billing",
                operation = "persist_billable_usage",
                event_id = %event.event_id,
                correlation_id = %event.correlation_id,
                duplicate_usage = duplicate
            );
            ack_message(&message, &event).await;
        }
        Err(UsageFailure::Permanent(code)) => {
            counter!("platform_billing_usage_events_total", "outcome" => "rejected").increment(1);
            match inbox::mark_rejected(pool, event.tenant_id, inbox_id, code).await {
                Ok(_) => term_message(
                    &message,
                    Some(event.event_id),
                    Some(&event.correlation_id),
                )
                .await,
                Err(error) => retry_message(&message, &event, &error).await,
            }
        }
        Err(UsageFailure::Retryable(error)) => {
            counter!("platform_billing_usage_events_total", "outcome" => "retry").increment(1);
            retry_message(&message, &event, &error).await;
        }
    }
}


struct UsageProcessingTimer(Instant);

impl Drop for UsageProcessingTimer {
    fn drop(&mut self) {
        histogram!("platform_billing_usage_processing_seconds")
            .record(self.0.elapsed().as_secs_f64());
    }
}

enum UsageFailure {
    Permanent(&'static str),
    Retryable(sqlx::Error),
}

async fn persist_usage(
    service: &BillingService,
    event: &UsageEvent,
    inbox_id: Uuid,
) -> Result<bool, UsageFailure> {
    let source_event_id = event.usage_id.to_string();
    let attributes = json!({
        "price_version_id": event.price_version_id,
        "environment": event.environment,
        "period_start": event.period_start,
        "period_end": event.period_end
    });
    service
        .record_usage_with_inbox(
            RecordUsage {
                tenant_id: event.tenant_id,
                meter_key: &event.meter_key,
                source_system: PRODUCER,
                source_event_id: &source_event_id,
                quantity: event.quantity,
                unit: &event.unit,
                occurred_at: event.occurred_at,
                attributes,
            },
            inbox_id,
        )
        .await
        .map(|(_, duplicate)| duplicate)
        .map_err(|error| match error {
            BillingError::Database(database) => UsageFailure::Retryable(database),
            BillingError::IdempotencyConflict => {
                UsageFailure::Permanent("usage_idempotency_conflict")
            }
            BillingError::InvalidCommand => UsageFailure::Permanent("billable_usage_invalid"),
            _ => UsageFailure::Permanent("billable_usage_rejected"),
        })
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ()> {
    value.get(key).and_then(Value::as_str).ok_or(())
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
    let event_id = Uuid::parse_str(required_string(&envelope, "event_id")?).map_err(|_| ())?;
    let tenant_id = Uuid::parse_str(required_string(payload, "tenant_id")?).map_err(|_| ())?;
    let usage_id = Uuid::parse_str(required_string(payload, "usage_id")?).map_err(|_| ())?;
    let price_version_id =
        Uuid::parse_str(required_string(payload, "price_version_id")?).map_err(|_| ())?;
    let quantity = payload
        .get("quantity")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or(())?;
    let environment = required_string(payload, "environment")?.to_owned();
    if !matches!(environment.as_str(), "test" | "live") {
        return Err(());
    }
    let occurred_at = DateTime::parse_from_rfc3339(required_string(payload, "occurred_at")?)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| ())?;
    let period_start = required_string(payload, "period_start")?.to_owned();
    let period_end = required_string(payload, "period_end")?.to_owned();
    let start = DateTime::parse_from_rfc3339(&period_start).map_err(|_| ())?;
    let end = DateTime::parse_from_rfc3339(&period_end).map_err(|_| ())?;
    if end <= start {
        return Err(());
    }
    let meter_key = required_string(payload, "meter_key")?.to_owned();
    let unit = required_string(payload, "unit")?.to_owned();
    let correlation_id = required_string(&envelope, "correlation_id")?.to_owned();
    if meter_key.is_empty()
        || meter_key.len() > 96
        || unit.is_empty()
        || unit.len() > 32
        || !payvia_observability::valid_correlation_id(&correlation_id)
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

async fn ack_message(message: &jetstream::Message, event: &UsageEvent) {
    if let Err(error) = message.ack().await {
        tracing::warn!(
            event.name = "jetstream.consumer.ack.failed",
            component = "platform_billing",
            operation = "ack_billable_usage",
            event_id = %event.event_id,
            correlation_id = %event.correlation_id,
            error_code = "billing_usage_ack_failed",
            error_class = %error
        );
    }
}

async fn term_message(
    message: &jetstream::Message,
    event_id: Option<Uuid>,
    correlation_id: Option<&str>,
) {
    if let Err(error) = message.ack_with(jetstream::AckKind::Term).await {
        tracing::error!(
            event.name = "jetstream.consumer.term.failed",
            component = "platform_billing",
            operation = "term_invalid_billable_usage",
            event_id = ?event_id,
            correlation_id = ?correlation_id,
            error_code = "billing_usage_term_failed",
            error_class = %error
        );
    }
}

async fn retry_message(
    message: &jetstream::Message,
    event: &UsageEvent,
    error: &sqlx::Error,
) {
    let code = error.as_database_error().and_then(|cause| cause.code());
    tracing::error!(
        event.name = "jetstream.consumer.processing.failed",
        component = "platform_billing",
        operation = "persist_billable_usage",
        dependency = "postgresql",
        event_id = %event.event_id,
        correlation_id = %event.correlation_id,
        error_code = code.as_deref().unwrap_or("billing_usage_persistence_failed"),
        error_category = "persistence",
        retryable = true
    );
    if let Err(ack_error) = message
        .ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5))))
        .await
    {
        tracing::error!(
            event.name = "jetstream.consumer.nak.failed",
            component = "platform_billing",
            operation = "retry_billable_usage",
            event_id = %event.event_id,
            correlation_id = %event.correlation_id,
            error_code = "billing_usage_nak_failed",
            error_class = %ack_error
        );
    }
}
