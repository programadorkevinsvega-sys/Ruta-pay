use std::{sync::Arc, time::Duration};

use async_nats::jetstream::{self, consumer::AckPolicy};
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{sync::watch, time};
use tokio_stream::StreamExt;
use uuid::Uuid;

mod processing;

use crate::{
    webhooks::{GatewayWebhookState, WompiAccountConfig},
    wompi::{WompiClient, WompiClientError, WompiEnvironment, WompiPaymentStatus, reference_for_attempt},
};

const PLATFORM_REQUEST_SUBJECT: &str =
    "payvia.platform.events.v1.payment.reconciliation.requested";
const PLATFORM_RESULT_SUBJECT: &str =
    "payvia.platform.events.v1.payments.provider_reconciliation.result";
const DURABLE_CONSUMER: &str = "GATEWAY_PAYMENT_RECONCILIATION";

#[derive(Clone)]
pub struct ReconciliationWorkerConfig {
    pub platform_request_key: Arc<[u8]>,
    pub platform_request_key_id: Arc<str>,
}

#[derive(Deserialize)]
struct PlatformEvent {
    event_id: Uuid,
    event_type: String,
    audience: String,
    correlation_id: String,
    payload: Value,
    producer_auth: ProducerAuth,
}

#[derive(Deserialize)]
struct ProducerAuth {
    key_id: String,
    signature: String,
}

#[derive(Deserialize)]
struct ReconciliationRequest {
    tenant_id: Uuid,
    environment: String,
    payment_intent_id: Uuid,
    attempt_id: Uuid,
    reconciliation_case_id: Uuid,
    provider_code: Option<String>,
    provider_account_reference: Option<String>,
    provider_transaction_reference: Option<String>,
    amount_minor: Option<i64>,
    currency: Option<String>,
}

#[derive(Serialize)]
struct ReconciliationResult<'a> {
    schema_version: u32,
    event_id: Uuid,
    event_type: &'static str,
    provider_code: &'static str,
    provider_account_reference: &'a str,
    tenant_id: Uuid,
    environment: &'a str,
    payment_intent_id: Uuid,
    payment_attempt_id: Uuid,
    reconciliation_case_id: Uuid,
    provider_transaction_reference: Option<&'a str>,
    outcome: &'static str,
    status: Option<&'static str>,
    amount_minor: Option<i64>,
    currency: Option<&'a str>,
    evidence_sha256: Option<String>,
    safe_error_code: Option<&'static str>,
    correlation_id: &'a str,
    occurred_at_unix_millis: i64,
}

#[derive(Serialize)]
struct SignedResult<'a> {
    key_id: &'a str,
    event: &'a ReconciliationResult<'a>,
    signature: String,
}

#[derive(Debug)]
enum RequestError {
    InvalidEnvelope,
    Unauthenticated,
    InvalidAccount,
    Unavailable,
}

pub async fn run(
    state: GatewayWebhookState,
    jetstream: jetstream::Context,
    config: ReconciliationWorkerConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let stream = match jetstream.get_stream("PAYVIA_PLATFORM_EVENTS").await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(
                    event.name = "gateway.reconciliation.stream.unavailable",
                    component = "gateway_reconciliation",
                    operation = "get_platform_event_stream",
                    dependency = "nats_jetstream",
                    error_code = "platform_event_stream_unavailable",
                    error_category = "dependency",
                    retryable = true,
                    error_class = %error
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        let consumer = match stream
            .get_or_create_consumer(
                DURABLE_CONSUMER,
                jetstream::consumer::pull::Config {
                    durable_name: Some(DURABLE_CONSUMER.to_owned()),
                    filter_subject: PLATFORM_REQUEST_SUBJECT.to_owned(),
                    deliver_policy: jetstream::consumer::DeliverPolicy::All,
                    ack_policy: AckPolicy::Explicit,
                    ack_wait: Duration::from_secs(60),
                    max_deliver: -1,
                    max_ack_pending: 32,
                    ..Default::default()
                },
            )
            .await
        {
            Ok(consumer) => consumer,
            Err(error) => {
                tracing::error!(
                    event.name = "gateway.reconciliation.consumer.unavailable",
                    component = "gateway_reconciliation",
                    operation = "create_durable_consumer",
                    dependency = "nats_jetstream",
                    error_code = "reconciliation_consumer_unavailable",
                    error_category = "dependency",
                    retryable = true,
                    error_class = %error
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        tracing::info!(
            event.name = "gateway.reconciliation.worker.started",
            component = "gateway_reconciliation",
            consumer = DURABLE_CONSUMER,
            result = "connected"
        );
        backoff = Duration::from_secs(1);
        loop {
            let mut messages = match consumer.messages().await {
                Ok(messages) => messages,
                Err(error) => {
                    tracing::error!(
                        event.name = "gateway.reconciliation.fetch.failed",
                        component = "gateway_reconciliation",
                        operation = "fetch_reconciliation_requests",
                        dependency = "nats_jetstream",
                        error_code = "reconciliation_fetch_failed",
                        error_category = "connectivity",
                        retryable = true,
                        error_class = %error
                    );
                    break;
                }
            };
            loop {
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    message = messages.next() => match message {
                        Some(Ok(message)) => {
                            let delivery_attempt = message
                                .info()
                                .ok()
                                .and_then(|info| i64::try_from(info.delivered).ok())
                                .unwrap_or(1);
                            match processing::process_request(
                                &state,
                                &jetstream,
                                &config,
                                &message.payload,
                                delivery_attempt,
                            ).await {
                                Ok(()) => {
                                    if let Err(error) = message.ack().await {
                                        tracing::error!(
                                            event.name = "gateway.reconciliation.ack.failed",
                                            component = "gateway_reconciliation",
                                            operation = "ack_reconciliation_request",
                                            dependency = "nats_jetstream",
                                            error_code = "reconciliation_ack_failed",
                                            error_category = "connectivity",
                                            retryable = true,
                                            error_class = %error
                                        );
                                    }
                                }
                                Err(RequestError::InvalidEnvelope) => {
                                    tracing::error!(
                                        event.name = "gateway.reconciliation.request.invalid",
                                        component = "gateway_reconciliation",
                                        operation = "validate_reconciliation_request",
                                        error_code = "reconciliation_request_invalid",
                                        error_category = "validation",
                                        retryable = false
                                    );
                                    let _ = message.ack_with(jetstream::AckKind::Term).await;
                                }
                                Err(RequestError::Unauthenticated) => {
                                    tracing::error!(
                                        event.name = "gateway.reconciliation.request.unauthenticated",
                                        component = "gateway_reconciliation",
                                        operation = "verify_platform_request",
                                        error_code = "platform_request_signature_invalid",
                                        error_category = "authorization",
                                        retryable = false
                                    );
                                    let _ = message.ack_with(jetstream::AckKind::Term).await;
                                }
                                Err(RequestError::InvalidAccount) => {
                                    tracing::error!(
                                        event.name = "gateway.reconciliation.account.unavailable",
                                        component = "gateway_reconciliation",
                                        operation = "resolve_provider_account",
                                        error_code = "provider_account_unavailable",
                                        error_category = "authorization",
                                        retryable = false
                                    );
                                    let _ = message.ack_with(jetstream::AckKind::Term).await;
                                }
                                Err(RequestError::Unavailable) => {
                                    if let Err(error) = message
                                        .ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(15))))
                                        .await
                                    {
                                        tracing::error!(
                                            event.name = "gateway.reconciliation.nak.failed",
                                            component = "gateway_reconciliation",
                                            operation = "reschedule_reconciliation_request",
                                            dependency = "nats_jetstream",
                                            error_code = "reconciliation_nak_failed",
                                            error_category = "connectivity",
                                            retryable = true,
                                            error_class = %error
                                        );
                                    }
                                }
                            }
                        }
                        Some(Err(error)) => {
                            tracing::warn!(
                                event.name = "gateway.reconciliation.stream.failed",
                                component = "gateway_reconciliation",
                                operation = "read_reconciliation_requests",
                                dependency = "nats_jetstream",
                                error_code = "reconciliation_stream_failed",
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
}

