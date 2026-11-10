use async_nats::jetstream;
use chrono::{DateTime, Utc};
use metrics::{counter, histogram};
use platform_billing::application::{
    BillingError, BillingService, RecordUsage, UsageBatchDisposition, UsageBatchEvent,
};
use serde_json::json;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::task::JoinSet;
use uuid::Uuid;

const EVENT_TYPE: &str = "billing.usage.recorded";
const PRODUCER: &str = "entitlements_quotas";

mod parser;
use parser::parse_event;

/// Groups a bounded fetch by tenant and runs tenant transactions concurrently
/// with a strict limit derived from currently idle PostgreSQL connections.
pub(super) async fn process_batch(
    messages: Vec<jetstream::Message>,
    service: &BillingService,
    max_concurrency: usize,
) {
    let mut groups: HashMap<Uuid, Vec<(jetstream::Message, UsageBatchEvent)>> = HashMap::new();

    for message in messages {
        record_queue_wait(&message);
        let event = match parse_event(&message.payload) {
            Ok(event) => event,
            Err(()) => {
                counter!("platform_billing_usage_events_total", "outcome" => "invalid")
                    .increment(1);
                term_message(&message, None, None).await;
                continue;
            }
        };
        tracing::debug!(
            event.name = "billing.usage.event.accepted",
            component = "platform_billing",
            operation = "parse_billable_usage_event",
            event_id = %event.event_id,
            tenant_id = %event.tenant_id,
            correlation_id = %event.correlation_id
        );
        let attributes = json!({
            "price_version_id": event.price_version_id,
            "environment": event.environment,
            "period_start": event.period_start,
            "period_end": event.period_end
        });
        let tenant_id = event.tenant_id;
        let batch_event = UsageBatchEvent {
            tenant_id,
            producer: PRODUCER.to_owned(),
            event_id: event.event_id,
            event_type: EVENT_TYPE.to_owned(),
            schema_version: 1,
            payload: event.envelope,
            source_system: PRODUCER.to_owned(),
            source_event_id: event.usage_id.to_string(),
            meter_key: event.meter_key,
            quantity: event.quantity,
            unit: event.unit,
            occurred_at: event.occurred_at,
            attributes,
        };
        groups
            .entry(tenant_id)
            .or_default()
            .push((message, batch_event));
    }

    histogram!("platform_billing_usage_tenant_groups_per_fetch").record(groups.len() as f64);
    let mut pending = groups.into_iter();
    let concurrency = max_concurrency.max(1).min(pending.len());
    let mut tasks = JoinSet::new();

    for _ in 0..concurrency {
        if let Some((tenant_id, group)) = pending.next() {
            let service = service.clone();
            tasks.spawn(async move {
                process_tenant_group(tenant_id, group, &service).await;
            });
        }
    }

    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            counter!("platform_billing_usage_tenant_task_failures_total").increment(1);
            tracing::error!(
                event.name = "billing.usage.tenant_task.failed",
                component = "platform_billing",
                operation = "process_tenant_usage_group",
                error_code = "billing_usage_tenant_task_failed",
                error_class = %error
            );
        }
        if let Some((tenant_id, group)) = pending.next() {
            let service = service.clone();
            tasks.spawn(async move {
                process_tenant_group(tenant_id, group, &service).await;
            });
        }
    }
}

async fn process_tenant_group(
    tenant_id: Uuid,
    group: Vec<(jetstream::Message, UsageBatchEvent)>,
    service: &BillingService,
) {
    let events = group
        .iter()
        .map(|(_, event)| event.clone())
        .collect::<Vec<_>>();
    match service
        .record_usage_batch_for_tenant(tenant_id, &events)
        .await
    {
        Ok(dispositions) => {
            for ((message, event), disposition) in group.into_iter().zip(dispositions) {
                let outcome = match disposition {
                    UsageBatchDisposition::Processed => "processed",
                    UsageBatchDisposition::DuplicateProcessed => "duplicate_processed",
                };
                counter!("platform_billing_usage_events_total", "outcome" => outcome).increment(1);
                let ack_started = Instant::now();
                if let Err(error) = message.ack().await {
                    counter!("platform_billing_usage_ack_failures_total").increment(1);
                    tracing::warn!(
                        event.name = "jetstream.consumer.ack.failed",
                        component = "platform_billing",
                        operation = "ack_committed_usage_batch",
                        event_id = %event.event_id,
                        error_code = "billing_usage_batch_ack_failed",
                        error_class = %error
                    );
                }
                histogram!("platform_billing_usage_ack_seconds")
                    .record(ack_started.elapsed().as_secs_f64());
            }
        }
        Err(BillingError::Database(database_error)) => {
            let code = database_error
                .as_database_error()
                .and_then(|cause| cause.code());
            tracing::error!(
                event.name = "billing.usage.batch.failed",
                component = "platform_billing",
                operation = "persist_billable_usage_batch",
                tenant_id = %tenant_id,
                batch_size = events.len(),
                error_code = code.as_deref().unwrap_or("billing_usage_batch_failed"),
                retryable = true
            );
            for (message, _) in group {
                counter!("platform_billing_usage_events_total", "outcome" => "retry").increment(1);
                nak_message(&message).await;
            }
        }
        Err(error) => {
            tracing::warn!(
                event.name = "billing.usage.batch.fallback",
                component = "platform_billing",
                operation = "persist_billable_usage_batch",
                tenant_id = %tenant_id,
                batch_size = events.len(),
                error_code = match error {
                    BillingError::InboxIdempotencyConflict => "inbox_idempotency_conflict",
                    BillingError::IdempotencyConflict => "usage_idempotency_conflict",
                    BillingError::InvalidCommand => "usage_batch_invalid",
                    _ => "usage_batch_rejected"
                }
            );
            for (message, event) in group {
                process_single_fallback(message, event, service).await;
            }
        }
    }
}

fn record_queue_wait(message: &jetstream::Message) {
    if let Ok(info) = message.info()
        && let Ok(published) = DateTime::parse_from_rfc3339(&info.published.to_string())
    {
        let age = Utc::now().signed_duration_since(published.with_timezone(&Utc));
        if let Ok(wait) = age.to_std() {
            histogram!("platform_billing_usage_queue_wait_seconds").record(wait.as_secs_f64());
        }
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

async fn process_single_fallback(
    message: jetstream::Message,
    event: UsageBatchEvent,
    service: &BillingService,
) {
    let result = service
        .record_usage_from_event(
            RecordUsage {
                tenant_id: event.tenant_id,
                meter_key: &event.meter_key,
                source_system: &event.source_system,
                source_event_id: &event.source_event_id,
                quantity: event.quantity,
                unit: &event.unit,
                occurred_at: event.occurred_at,
                attributes: event.attributes,
            },
            &event.producer,
            event.event_id,
            &event.event_type,
            event.schema_version,
            &event.payload,
        )
        .await;
    match result {
        Ok(_) => {
            counter!("platform_billing_usage_events_total", "outcome" => "fallback_processed")
                .increment(1);
            let ack_started = Instant::now();
            if let Err(error) = message.ack().await {
                counter!("platform_billing_usage_ack_failures_total").increment(1);
                tracing::warn!(
                    event.name = "jetstream.consumer.ack.failed",
                    component = "platform_billing",
                    operation = "ack_fallback_committed_usage",
                    event_id = %event.event_id,
                    error_code = "billing_usage_fallback_ack_failed",
                    error_class = %error
                );
            }
            histogram!("platform_billing_usage_ack_seconds")
                .record(ack_started.elapsed().as_secs_f64());
        }
        Err(BillingError::Database(error)) => {
            tracing::error!(
                event.name = "billing.usage.fallback.failed",
                component = "platform_billing",
                operation = "persist_billable_usage_fallback",
                event_id = %event.event_id,
                error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("billing_usage_fallback_failed"),
                retryable = true
            );
            nak_message(&message).await;
        }
        Err(_) => {
            counter!("platform_billing_usage_events_total", "outcome" => "fallback_rejected")
                .increment(1);
            term_message(&message, Some(event.event_id), None).await;
        }
    }
}

async fn nak_message(message: &jetstream::Message) {
    if let Err(error) = message
        .ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5))))
        .await
    {
        tracing::error!(
            event.name = "jetstream.consumer.nak.failed",
            component = "platform_billing",
            operation = "retry_billable_usage_event",
            error_code = "billing_usage_nak_failed",
            error_class = %error
        );
    }
}
