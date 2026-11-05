use async_nats::jetstream::{self, consumer::AckPolicy};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use platform_payments::application::payment_service::{
    GatewayEventDisposition, PaymentService, PaymentServiceError,
};
use sqlx::PgPool;
use std::{env, time::Duration};
use tokio::{sync::watch, time};
use tokio_stream::StreamExt;

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
    let result_subject =
        format!("{subject_prefix}.payments.provider_reconciliation.result");
    let provider_consumer = match ensure_consumer(
        &stream,
        PROVIDER_EVENT_CONSUMER,
        &provider_subject,
    )
    .await
    {
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
    let reconciliation_consumer = match ensure_consumer(
        &stream,
        RECONCILIATION_RESULT_CONSUMER,
        &result_subject,
    )
    .await
    {
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
                        match service
                            .ingest_signed_provider_event(&message.payload, &key, &key_id)
                            .await
                        {
                            Ok(disposition) => {
                                tracing::info!(
                                    event.name = "payments.provider_event.processed",
                                    component = "payments",
                                    operation = "ingest_provider_event",
                                    disposition = ?disposition
                                );
                                acknowledge(message).await;
                            }
                            Err(error) => reject_or_retry(message, error).await,
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
                        match service
                            .apply_signed_reconciliation_result(&message.payload, &key, &key_id)
                            .await
                        {
                            Ok(disposition) => {
                                tracing::info!(
                                    event.name = "payments.reconciliation.result.processed",
                                    component = "payments",
                                    operation = "apply_reconciliation_result",
                                    disposition = ?disposition
                                );
                                acknowledge(message).await;
                            }
                            Err(error) => reject_or_retry(message, error).await,
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

async fn acknowledge(message: jetstream::Message) {
    if let Err(error) = message.ack().await {
        tracing::error!(
            event.name = "payments.gateway_event.ack.failed",
            component = "payments",
            operation = "acknowledge_gateway_event",
            dependency = "nats_jetstream",
            error_code = "gateway_event_ack_failed",
            error_category = "connectivity",
            retryable = true,
            error_class = %error
        );
    }
}

async fn reject_or_retry(message: jetstream::Message, error: PaymentServiceError) {
    let permanent = matches!(
        error,
        PaymentServiceError::ProviderEventInvalid
            | PaymentServiceError::Serialization(_)
    );
    tracing::error!(
        event.name = "payments.gateway_event.processing.failed",
        component = "payments",
        operation = "apply_gateway_event",
        error_category = if permanent { "validation" } else { "dependency" },
        error_code = if permanent { "gateway_event_invalid" } else { "gateway_event_processing_failed" },
        retryable = !permanent
    );
    let acknowledgement = if permanent {
        message.ack_with(jetstream::AckKind::Term).await
    } else {
        message
            .ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5))))
            .await
    };
    if let Err(error) = acknowledgement {
        tracing::error!(
            event.name = "payments.gateway_event.negative_ack.failed",
            component = "payments",
            operation = "negative_ack_gateway_event",
            dependency = "nats_jetstream",
            error_code = "gateway_event_negative_ack_failed",
            error_category = "connectivity",
            retryable = true,
            error_class = %error
        );
    }
}

fn load_verification_key() -> Result<Vec<u8>, &'static str> {
    let encoded = env::var("PLATFORM_GATEWAY_EVENT_VERIFY_KEY")
        .map_err(|_| "gateway_event_verify_key_missing")?;
    let key = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "gateway_event_verify_key_invalid")?;
    if key.len() < 32 {
        return Err("gateway_event_verify_key_invalid");
    }
    Ok(key)
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}
