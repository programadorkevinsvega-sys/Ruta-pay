use async_nats::jetstream;
use metrics::{counter, histogram};
use platform_billing::{
    application::{BillingError, BillingService, RecordUsage},
    infrastructure::inbox::{self, InboxDisposition},
};
use serde_json::json;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;
use tracing::Instrument;

const EVENT_TYPE: &str = "billing.usage.recorded";
const PRODUCER: &str = "entitlements_quotas";

mod parser;
use parser::{parse_event, UsageEvent};

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
    let span = tracing::info_span!(
        "billing_usage.process",
        event_id = %event.event_id,
        correlation_id = %event.correlation_id
    );
    process_valid_message(message, service, pool, event)
        .instrument(span)
        .await;
}

async fn process_valid_message(
    message: jetstream::Message,
    service: &BillingService,
    pool: &PgPool,
    event: UsageEvent,
) {
    tracing::debug!(
        event.name = "billing.usage.event.received",
        component = "platform_billing",
        operation = "consume_billable_usage",
        event_id = %event.event_id,
        correlation_id = %event.correlation_id
    );

    let source_event_id = event.usage_id.to_string();
    let attributes = json!({
        "price_version_id": event.price_version_id,
        "environment": event.environment,
        "period_start": event.period_start,
        "period_end": event.period_end
    });
    let result = service.record_usage_from_event(
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
        PRODUCER,
        event.event_id,
        EVENT_TYPE,
        1,
        &event.envelope,
    ).await;

    match result {
        Ok(platform_billing::application::UsageInboxResult::Processed { duplicate_usage }) => {
            let outcome = if duplicate_usage { "duplicate_usage" } else { "processed" };
            counter!("platform_billing_usage_events_total", "outcome" => outcome).increment(1);
            tracing::info!(
                event.name = "billing.usage.event.processed",
                component = "platform_billing",
                operation = "persist_billable_usage",
                event_id = %event.event_id,
                correlation_id = %event.correlation_id,
                duplicate_usage
            );
            ack_message(&message, &event).await;
        }
        Ok(platform_billing::application::UsageInboxResult::DuplicateProcessed) => {
            counter!("platform_billing_usage_events_total", "outcome" => "duplicate_processed").increment(1);
            ack_message(&message, &event).await;
        }
        Err(BillingError::Database(error)) => {
            counter!("platform_billing_usage_events_total", "outcome" => "retry").increment(1);
            retry_message(&message, &event, &error).await;
        }
        Err(BillingError::InboxIdempotencyConflict) => {
            counter!("platform_billing_usage_events_total", "outcome" => "idempotency_conflict").increment(1);
            term_message(&message, Some(event.event_id), Some(&event.correlation_id)).await;
        }
        Err(BillingError::IdempotencyConflict) => {
            counter!("platform_billing_usage_events_total", "outcome" => "usage_idempotency_conflict").increment(1);
            reject_event(pool, &message, &event, "usage_idempotency_conflict").await;
        }
        Err(BillingError::InvalidCommand) => {
            counter!("platform_billing_usage_events_total", "outcome" => "rejected").increment(1);
            reject_event(pool, &message, &event, "billable_usage_invalid").await;
        }
        Err(_) => {
            counter!("platform_billing_usage_events_total", "outcome" => "rejected").increment(1);
            reject_event(pool, &message, &event, "billable_usage_rejected").await;
        }
    }
}

async fn reject_event(
    pool: &PgPool,
    message: &jetstream::Message,
    event: &UsageEvent,
    code: &'static str,
) {
    match inbox::receive(
        pool,
        event.tenant_id,
        PRODUCER,
        event.event_id,
        EVENT_TYPE,
        1,
        &event.envelope,
    ).await {
        Ok(InboxDisposition::Inserted(id)) | Ok(InboxDisposition::Duplicate { id, processed: false }) => {
            match inbox::mark_rejected(pool, event.tenant_id, id, code).await {
                Ok(_) => term_message(message, Some(event.event_id), Some(&event.correlation_id)).await,
                Err(error) => retry_message(message, event, &error).await,
            }
        }
        Ok(InboxDisposition::Duplicate { processed: true, .. }) => ack_message(message, event).await,
        Err(error) => {
            if matches!(&error, sqlx::Error::Protocol(_)) {
                term_message(message, Some(event.event_id), Some(&event.correlation_id)).await;
            } else {
                retry_message(message, event, &error).await;
            }
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
