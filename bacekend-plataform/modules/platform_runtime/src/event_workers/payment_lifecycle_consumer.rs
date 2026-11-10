use async_nats::jetstream::{self, consumer::AckPolicy};
use platform_payments::application::payment_service::{GatewayEventDisposition, PaymentService};
use sqlx::PgPool;
use std::time::Duration;
use tokio::{sync::watch, time};
use tokio_stream::StreamExt;
use tracing::Instrument;

mod diagnostics;

use diagnostics::{
    acknowledge, consumer_span, load_verification_key, message_context, reject_or_retry,
    wait_for_shutdown,
};

const PROVIDER_EVENT_CONSUMER: &str = "PLATFORM_PAYMENT_PROVIDER_EVENTS";
const RECONCILIATION_RESULT_CONSUMER: &str = "PLATFORM_PAYMENT_RECONCILIATION_RESULTS";

pub(super) async fn run(
    pool: PgPool,
    stream: jetstream::stream::Stream,
    subject_prefix: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let key = match load_verification_key() {
        Ok(key) => key,
        Err(error_code) => {
            tracing::error!(
                event.name = "service.configuration.invalid",
                component = "payments",
                operation = "configure_gateway_event_verification",
                error_code,
                error_category = "configuration",
                retryable = false
            );
            wait_for_shutdown(&mut shutdown).await;
            return;
        }
    };
    let key_id = match env::var("PLATFORM_GATEWAY_EVENT_VERIFY_KEY_ID") {
        Ok(value) if !value.is_empty() && value.len() <= 64 => value,
        _ => {
            tracing::error!(
                event.name = "service.configuration.invalid",
                component = "payments",
                operation = "configure_gateway_event_verification",
                error_code = "gateway_event_verify_key_id_invalid",
                error_category = "configuration",
                retryable = false
            );
            wait_for_shutdown(&mut shutdown).await;
            return;
        }
    };
    let provider_subject = format!("{subject_prefix}.payments.provider_event.verified");
    let result_subject = format!("{subject_prefix}.payments.provider_reconciliation.result");
    let provider_consumer =
        match ensure_consumer(&stream, PROVIDER_EVENT_CONSUMER, &provider_subject).await {
            Ok(consumer) => consumer,
            Err(error) => {
                tracing::error!(
                    event.name = "payments.provider_event.consumer.unavailable",
                    component = "payments",
                    operation = "ensure_provider_event_consumer",
                    dependency = "nats_jetstream",
                    error_code = "provider_event_consumer_unavailable",
                    error_category = "dependency",
                    retryable = true,
                    error_class = %error
                );
                return;
            }
        };
    let reconciliation_consumer =
        match ensure_consumer(&stream, RECONCILIATION_RESULT_CONSUMER, &result_subject).await {
            Ok(consumer) => consumer,
            Err(error) => {
                tracing::error!(
                    event.name = "payments.reconciliation.consumer.unavailable",
                    component = "payments",
                    operation = "ensure_reconciliation_result_consumer",
                    dependency = "nats_jetstream",
                    error_code = "reconciliation_result_consumer_unavailable",
                    error_category = "dependency",
                    retryable = true,
                    error_class = %error
                );
                return;
            }
        };
    let service = PaymentService::new(pool);
    tracing::info!(
        event.name = "payments.gateway_consumers.started",
        component = "payments",
        provider_event_consumer = PROVIDER_EVENT_CONSUMER,
        reconciliation_consumer = RECONCILIATION_RESULT_CONSUMER
    );
    tokio::select! {
        _ = run_provider_events(
            service.clone(),
            provider_consumer,
            key.clone(),
            key_id.clone(),
            shutdown.clone(),
        ) => {},
        _ = run_reconciliation_results(
            service,
            reconciliation_consumer,
            key,
            key_id,
            shutdown.clone(),
        ) => {},
        result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; }
    }
}

async fn ensure_consumer(
    stream: &jetstream::stream::Stream,
    name: &str,
    subject: &str,
) -> Result<jetstream::consumer::PullConsumer, String> {
    stream
        .get_or_create_consumer(
            name,
            jetstream::consumer::pull::Config {
                durable_name: Some(name.to_owned()),
                filter_subject: subject.to_owned(),
                deliver_policy: jetstream::consumer::DeliverPolicy::All,
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(60),
                max_deliver: 20,
                max_ack_pending: 256,
                ..Default::default()
            },
        )
        .await
        .map_err(|error| error.to_string())
}

async fn run_provider_events(
    service: PaymentService,
    consumer: jetstream::consumer::PullConsumer,
    key: Vec<u8>,
    key_id: String,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(error) => {
                tracing::error!(
                    event.name = "payments.provider_event.fetch.failed",
                    component = "payments",
                    operation = "fetch_provider_events",
                    dependency = "nats_jetstream",
                    error_code = "provider_event_fetch_failed",
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
                        let (traceparent, correlation_id) = message_context(&message.payload);
                        let span = consumer_span("payments.provider_event.consume", &correlation_id, traceparent.as_deref());
                        let started_at = std::time::Instant::now();
                        let result = service
                            .ingest_signed_provider_event(&message.payload, &key, &key_id)
                            .instrument(span.clone())
                            .await;
                        match result {
                            Ok(disposition) => {
                                tracing::info!(
                                    event.name = "payments.provider_event.processed",
                                    component = "payments",
                                    operation = "ingest_provider_event",
                                    disposition = ?disposition,
                                    correlation_id = %correlation_id
                                );
                                metrics::counter!("payvia_nats_messages_processed_total", "consumer" => "payment_provider_event", "result" => "success").increment(1);
                                metrics::histogram!("payvia_nats_processing_duration_seconds", "consumer" => "payment_provider_event").record(started_at.elapsed().as_secs_f64());
                                acknowledge(message).instrument(span).await;
                            }
                            Err(error) => {
                                metrics::counter!("payvia_nats_messages_processed_total", "consumer" => "payment_provider_event", "result" => "error").increment(1);
                                metrics::histogram!("payvia_nats_processing_duration_seconds", "consumer" => "payment_provider_event").record(started_at.elapsed().as_secs_f64());
                                reject_or_retry(message, error).instrument(span).await;
                            }
                        }
                    }
                    Some(Err(error)) => {
                        tracing::error!(
                            event.name = "payments.provider_event.stream.failed",
                            component = "payments",
                            operation = "read_provider_events",
                            dependency = "nats_jetstream",
                            error_code = "provider_event_stream_failed",
                            error_category = "connectivity",
                            retryable = true,
                            error_class = %error
                        );
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}

async fn run_reconciliation_results(
    service: PaymentService,
    consumer: jetstream::consumer::PullConsumer,
    key: Vec<u8>,
    key_id: String,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(error) => {
                tracing::error!(
                    event.name = "payments.reconciliation.fetch.failed",
                    component = "payments",
                    operation = "fetch_reconciliation_results",
                    dependency = "nats_jetstream",
                    error_code = "reconciliation_result_fetch_failed",
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
                        let (traceparent, correlation_id) = message_context(&message.payload);
                        let span = consumer_span("payments.reconciliation_result.consume", &correlation_id, traceparent.as_deref());
                        let started_at = std::time::Instant::now();
                        let result = service
                            .apply_signed_reconciliation_result(&message.payload, &key, &key_id)
                            .instrument(span.clone())
                            .await;
                        match result {
                            Ok(disposition) => {
                                tracing::info!(
                                    event.name = "payments.reconciliation.result.processed",
                                    component = "payments",
                                    operation = "apply_reconciliation_result",
                                    disposition = ?disposition,
                                    correlation_id = %correlation_id
                                );
                                metrics::counter!("payvia_nats_messages_processed_total", "consumer" => "payment_reconciliation_result", "result" => "success").increment(1);
                                metrics::histogram!("payvia_nats_processing_duration_seconds", "consumer" => "payment_reconciliation_result").record(started_at.elapsed().as_secs_f64());
                                acknowledge(message).instrument(span).await;
                            }
                            Err(error) => {
                                metrics::counter!("payvia_nats_messages_processed_total", "consumer" => "payment_reconciliation_result", "result" => "error").increment(1);
                                metrics::histogram!("payvia_nats_processing_duration_seconds", "consumer" => "payment_reconciliation_result").record(started_at.elapsed().as_secs_f64());
                                reject_or_retry(message, error).instrument(span).await;
                            }
                        }
                    }
                    Some(Err(error)) => {
                        tracing::error!(
                            event.name = "payments.reconciliation.stream.failed",
                            component = "payments",
                            operation = "read_reconciliation_results",
                            dependency = "nats_jetstream",
                            error_code = "reconciliation_result_stream_failed",
                            error_category = "connectivity",
                            retryable = true,
                            error_class = %error
                        );
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}
