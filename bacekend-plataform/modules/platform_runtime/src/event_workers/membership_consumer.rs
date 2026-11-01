use async_nats::jetstream::{self, consumer::AckPolicy};
use platform_identity_access::infrastructure::tenant_membership_consumer::{TenantMembershipConsumerError, consume_tenant_registered};
use sqlx::PgPool;
use std::time::Duration;
use tokio::{sync::watch, time};

pub(super) async fn run_membership_consumer(
    pool: PgPool,
    consumer: jetstream::consumer::PullConsumer,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(_) => {
                tracing::warn!(
                    event.name = "jetstream.consumer.fetch.failed",
                    component = "identity_access",
                    operation = "consume_tenant_registered",
                    dependency = "nats_jetstream",
                    error_code = "consumer_fetch_failed",
                    error_category = "connectivity",
                    retryable = true
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
                        match consume_tenant_registered(&pool, &message.payload).await {
                            Ok(processed) => {
                                if let Err(error) = message.ack().await {
                                    tracing::warn!(event.name = "jetstream.consumer.ack.failed", component = "identity_access", operation = "ack_tenant_registered", dependency = "nats_jetstream", error_code = "consumer_ack_failed", error_category = "connectivity", retryable = true, error_class = %error);
                                } else if processed {
                                    tracing::info!(event.name = "membership.owner.provisioned", component = "identity_access", operation = "consume_tenant_registered", result = "success");
                                }
                            }
                            Err(TenantMembershipConsumerError::InvalidEnvelope) => {
                                tracing::error!(event.name = "jetstream.consumer.message.invalid", component = "identity_access", operation = "consume_tenant_registered", dependency = "nats_jetstream", error_code = "tenant_event_invalid", error_category = "validation", retryable = false);
                                if let Err(error) = message.ack_with(jetstream::AckKind::Term).await {
                                    tracing::error!(event.name = "jetstream.consumer.term.failed", component = "identity_access", operation = "term_invalid_event", dependency = "nats_jetstream", error_code = "consumer_term_failed", error_category = "connectivity", retryable = true, error_class = %error);
                                }
                            }
                            Err(TenantMembershipConsumerError::Database(error)) => {
                                let sqlstate = error.as_database_error().and_then(|cause| cause.code());
                                let event_correlation = serde_json::from_slice::<serde_json::Value>(&message.payload)
                                    .ok()
                                    .and_then(|envelope| envelope.get("correlation_id").and_then(serde_json::Value::as_str).map(str::to_owned))
                                    .unwrap_or_else(|| "unavailable".to_owned());
                                let permanent = sqlstate.as_deref() == Some("23503");
                                tracing::error!(event.name = "jetstream.consumer.processing.failed", component = "identity_access", operation = "consume_tenant_registered", dependency = "postgresql", error_code = sqlstate.as_deref().unwrap_or("membership_persistence_failed"), error_category = "persistence", retryable = !permanent, correlation_id = %event_correlation);
                                let acknowledgement = if permanent {
                                    message.ack_with(jetstream::AckKind::Term).await
                                } else {
                                    message.ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5)))).await
                                };
                                if let Err(ack_error) = acknowledgement {
                                    tracing::error!(event.name = "jetstream.consumer.acknowledgement.failed", component = "identity_access", operation = "acknowledge_tenant_registered_failure", dependency = "nats_jetstream", error_code = "consumer_acknowledgement_failed", error_category = "connectivity", retryable = true, correlation_id = %event_correlation, error_class = %ack_error);
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(event.name = "jetstream.consumer.stream.failed", component = "identity_access", operation = "read_tenant_registered", dependency = "nats_jetstream", error_code = "consumer_stream_failed", error_category = "connectivity", retryable = true, error_class = %error);
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}
