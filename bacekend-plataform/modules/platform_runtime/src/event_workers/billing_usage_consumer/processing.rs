use async_nats::jetstream;
use metrics::{counter, histogram};
use platform_billing::application::{BillingError, BillingService, UsageBatchEvent};
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;
use chrono::{DateTime, Utc};

const EVENT_TYPE: &str = "billing.usage.recorded";
const PRODUCER: &str = "entitlements_quotas";

mod parser;
use parser::parse_event;

/// Groups a bounded JetStream fetch by tenant and persists each group atomically.
/// ACKs are issued only after the corresponding tenant transaction has committed.
pub(super) async fn process_batch(messages: Vec<jetstream::Message>, service: &BillingService) {
    let mut groups: HashMap<Uuid, Vec<(jetstream::Message, UsageBatchEvent)>> = HashMap::new();

    for message in messages {
        record_queue_wait(&message);
        let event = match parse_event(&message.payload) {
            Ok(event) => event,
            Err(()) => {
                counter!("platform_billing_usage_events_total", "outcome" => "invalid").increment(1);
                term_message(&message, None, None).await;
                continue;
            }
        };
        let attributes = json!({
            "price_version_id": event.price_version_id,
            "environment": event.environment,
            "period_start": event.period_start,
            "period_end": event.period_end
        });
        let batch_event = UsageBatchEvent {
            tenant_id: event.tenant_id,
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
        groups.entry(event.tenant_id).or_default().push((message, batch_event));
    }

    for (tenant_id, group) in groups {
        let events = group.iter().map(|(_, event)| event.clone()).collect::<Vec<_>>();
        match service.record_usage_batch_for_tenant(tenant_id, &events).await {
            Ok(dispositions) => {
                for ((message, event), disposition) in group.into_iter().zip(dispositions) {
                    let outcome = match disposition {
                        platform_billing::application::UsageBatchDisposition::Processed => "processed",
                        platform_billing::application::UsageBatchDisposition::DuplicateProcessed => "duplicate_processed",
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
            Err(error) => {
                for (message, event) in group {
                    match &error {
                        BillingError::Database(database_error) => {
                            counter!("platform_billing_usage_events_total", "outcome" => "retry").increment(1);
                            let code = database_error.as_database_error().and_then(|cause| cause.code());
                            tracing::error!(
                                event.name = "billing.usage.batch.failed",
                                component = "platform_billing",
                                operation = "persist_billable_usage_batch",
                                tenant_id = %tenant_id,
                                batch_size = events.len(),
                                error_code = code.as_deref().unwrap_or("billing_usage_batch_failed"),
                                retryable = true
                            );
                            if let Err(ack_error) = message.ack_with(
                                jetstream::AckKind::Nak(Some(Duration::from_secs(5)))
                            ).await {
                                tracing::error!(
                                    event.name = "jetstream.consumer.nak.failed",
                                    component = "platform_billing",
                                    operation = "retry_billable_usage_batch",
                                    error_code = "billing_usage_batch_nak_failed",
                                    error_class = %ack_error
                                );
                            }
                        }
                        _ => {
                            counter!("platform_billing_usage_events_total", "outcome" => "batch_rejected").increment(1);
                            term_message(&message, Some(event.event_id), None).await;
                        }
                    }
                }
            }
        }
    }
}


fn record_queue_wait(message: &jetstream::Message) {
    if let Ok(info) = message.info() {
        if let Ok(published) = DateTime::parse_from_rfc3339(&info.published.to_string()) {
            let age = Utc::now().signed_duration_since(published.with_timezone(&Utc));
            if let Ok(wait) = age.to_std() {
                histogram!("platform_billing_usage_queue_wait_seconds")
                    .record(wait.as_secs_f64());
            }
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
