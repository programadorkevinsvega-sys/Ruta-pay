use std::{collections::HashMap, sync::Arc};

use async_nats::{HeaderMap, jetstream};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap as HttpHeaders, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use payvia_observability::{current_traceparent, new_correlation_id, valid_correlation_id};
use ring::hmac;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::wompi::{WompiEnvironment, verify_event};

mod request_diagnostics;

use request_diagnostics::request_diagnostics;

#[cfg(test)]
mod tests;

const PLATFORM_SUBJECT: &str = "payvia.platform.events.v1.payments.provider_event.verified";
const MAX_CALLBACK_BYTES: usize = 65_536;

#[derive(Clone)]
pub struct GatewayWebhookState {
    pub(crate) accounts: Arc<HashMap<String, WompiAccountConfig>>,
    pub(crate) jetstream: jetstream::Context,
    pub(crate) event_signing_key: Arc<hmac::Key>,
    pub(crate) signing_key_id: Arc<str>,
}

#[derive(Deserialize)]
pub struct WompiAccountConfig {
    pub account_reference: String,
    pub tenant_id: Uuid,
    pub environment: String,
    pub event_secret: String,
    pub private_key: String,
}

#[derive(Serialize)]
struct ProviderEvent {
    schema_version: u32,
    provider_code: &'static str,
    provider_account_reference: String,
    provider_event_id: String,
    tenant_id: Uuid,
    environment: String,
    payment_attempt_id: Uuid,
    provider_transaction_reference: String,
    status: &'static str,
    amount_minor: i64,
    currency: String,
    occurred_at_unix_millis: i64,
    correlation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    traceparent: Option<String>,
    payload_sha256: String,
    event_id: Uuid,
}

#[derive(Serialize)]
struct SignedProviderEvent<'a> {
    key_id: &'a str,
    event: &'a ProviderEvent,
    signature: String,
}

#[derive(Debug)]
enum WebhookError {
    UnknownAccount,
    InvalidConfiguration,
    InvalidCallback,
    Unauthenticated,
    EventBusUnavailable,
}

impl IntoResponse for WebhookError {
    fn into_response(self) -> Response {
        let (status, diagnostic) = match self {
            Self::UnknownAccount => (
                StatusCode::NOT_FOUND,
                WebhookDiagnostic {
                    code: "provider_account_not_found",
                    category: "validation",
                    dependency: "none",
                    retryable: false,
                },
            ),
            Self::InvalidConfiguration => (
                StatusCode::SERVICE_UNAVAILABLE,
                WebhookDiagnostic {
                    code: "provider_account_unavailable",
                    category: "configuration",
                    dependency: "none",
                    retryable: false,
                },
            ),
            Self::InvalidCallback => (
                StatusCode::BAD_REQUEST,
                WebhookDiagnostic {
                    code: "provider_callback_invalid",
                    category: "validation",
                    dependency: "none",
                    retryable: false,
                },
            ),
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                WebhookDiagnostic {
                    code: "provider_callback_unauthenticated",
                    category: "authentication",
                    dependency: "none",
                    retryable: false,
                },
            ),
            Self::EventBusUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                WebhookDiagnostic {
                    code: "provider_event_delivery_unavailable",
                    category: "dependency",
                    dependency: "nats_jetstream",
                    retryable: true,
                },
            ),
        };
        let mut response = (
            status,
            Json(serde_json::json!({"error":{"code":diagnostic.code}})),
        )
            .into_response();
        response.extensions_mut().insert(diagnostic);
        response
    }
}

#[derive(Clone, Copy)]
pub(super) struct WebhookDiagnostic {
    code: &'static str,
    category: &'static str,
    dependency: &'static str,
    retryable: bool,
}

impl GatewayWebhookState {
    pub fn new(
        accounts: Vec<WompiAccountConfig>,
        jetstream: jetstream::Context,
        event_signing_key: &[u8],
        signing_key_id: String,
    ) -> Result<Self, &'static str> {
        if event_signing_key.len() < 32
            || signing_key_id.is_empty()
            || signing_key_id.len() > 64
            || !signing_key_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
        {
            return Err("gateway_event_signing_key_invalid");
        }
        let mut registry = HashMap::new();
        for account in accounts {
            validate_account(&account)?;
            let key = account.account_reference.clone();
            if registry.insert(key, account).is_some() {
                return Err("wompi_account_reference_duplicate");
            }
        }
        if registry.is_empty() {
            return Err("wompi_account_registry_empty");
        }
        Ok(Self {
            accounts: Arc::new(registry),
            jetstream,
            event_signing_key: Arc::new(hmac::Key::new(hmac::HMAC_SHA256, event_signing_key)),
            signing_key_id: Arc::from(signing_key_id),
        })
    }
}

pub fn router(state: GatewayWebhookState) -> Router {
    Router::new()
        .route("/health/ready", get(readiness))
        .route(
            "/v1/webhooks/wompi/{account_reference}",
            post(receive_wompi_event),
        )
        .layer(axum::extract::DefaultBodyLimit::max(MAX_CALLBACK_BYTES))
        .layer(axum::middleware::from_fn(request_diagnostics))
        .with_state(state)
}

async fn readiness() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn receive_wompi_event(
    State(state): State<GatewayWebhookState>,
    Path(account_reference): Path<String>,
    headers: HttpHeaders,
    body: Bytes,
) -> Result<StatusCode, WebhookError> {
    if body.is_empty() || body.len() > MAX_CALLBACK_BYTES {
        return Err(WebhookError::InvalidCallback);
    }
    let account = state
        .accounts
        .get(&account_reference)
        .ok_or(WebhookError::UnknownAccount)?;
    let environment = WompiEnvironment::parse(&account.environment)
        .map_err(|_| WebhookError::InvalidConfiguration)?;
    let checksum = headers
        .get("x-event-checksum")
        .and_then(|value| value.to_str().ok());
    let verified =
        verify_event(&body, checksum, &account.event_secret, environment).map_err(|error| {
            match error {
                crate::wompi::WompiEventError::InvalidSignature => WebhookError::Unauthenticated,
                _ => WebhookError::InvalidCallback,
            }
        })?;

    let event = ProviderEvent {
        schema_version: 1,
        provider_code: "wompi",
        provider_account_reference: account.account_reference.clone(),
        provider_event_id: verified.provider_event_id.clone(),
        tenant_id: account.tenant_id,
        environment: account.environment.clone(),
        payment_attempt_id: verified.attempt_id,
        provider_transaction_reference: verified.provider_transaction_reference,
        status: verified.status.as_platform_status(),
        amount_minor: verified.amount_minor,
        currency: verified.currency,
        occurred_at_unix_millis: verified.occurred_at.timestamp_millis(),
        correlation_id: headers
            .get("x-correlation-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_correlation_id(value))
            .map(str::to_owned)
            .unwrap_or_else(new_correlation_id),
        traceparent: current_traceparent(),
        payload_sha256: hex_upper(&verified.payload_sha256),
        event_id: verified.event_id,
    };
    let unsigned_value = serde_json::to_value(&event).map_err(|_| WebhookError::InvalidCallback)?;
    let unsigned =
        serde_json::to_vec(&unsigned_value).map_err(|_| WebhookError::InvalidCallback)?;
    let signature = hex_upper(hmac::sign(&state.event_signing_key, &unsigned).as_ref());
    let envelope = SignedProviderEvent {
        key_id: &state.signing_key_id,
        event: &event,
        signature,
    };
    let payload = serde_json::to_vec(&envelope).map_err(|_| WebhookError::InvalidCallback)?;
    let mut nats_headers = HeaderMap::new();
    nats_headers.append("Nats-Msg-Id", event.event_id.to_string());
    nats_headers.append("Payvia-Event-Type", "provider.payment_event.verified");
    nats_headers.append("Payvia-Producer", "gateway-pasarela");
    let acknowledgement = state
        .jetstream
        .publish_with_headers(PLATFORM_SUBJECT, nats_headers, payload.into())
        .await
        .map_err(|_| WebhookError::EventBusUnavailable)?;
    acknowledgement
        .await
        .map_err(|_| WebhookError::EventBusUnavailable)?;

    tracing::info!(
        event.name = "gateway.webhook.accepted",
        component = "gateway_wompi",
        operation = "verify_and_publish_callback",
        provider = "wompi",
        provider_account_reference = %account.account_reference,
        provider_event_id = %event.provider_event_id,
        event_id = %event.event_id,
        correlation_id = %event.correlation_id,
        result = "durably_published"
    );
    Ok(StatusCode::NO_CONTENT)
}

fn validate_account(account: &WompiAccountConfig) -> Result<(), &'static str> {
    if account.tenant_id.is_nil()
        || account.account_reference.is_empty()
        || account.account_reference.len() > 128
        || !account.account_reference.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
    {
        return Err("wompi_account_configuration_invalid");
    }
    let environment = WompiEnvironment::parse(&account.environment)?;
    let expected_prefix = match environment {
        WompiEnvironment::Test => "test_events_",
        WompiEnvironment::Live => "prod_events_",
    };
    if !account.event_secret.starts_with(expected_prefix) || account.event_secret.len() < 24 {
        return Err("wompi_event_secret_environment_mismatch");
    }
    let private_prefix = match environment {
        WompiEnvironment::Test => "prv_test_",
        WompiEnvironment::Live => "prv_prod_",
    };
    if !account.private_key.starts_with(private_prefix) || account.private_key.len() < 24 {
        return Err("wompi_private_key_environment_mismatch");
    }
    Ok(())
}

fn hex_upper(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn load_event_signing_key(encoded: &str) -> Result<Vec<u8>, &'static str> {
    let key = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "gateway_event_signing_key_invalid")?;
    if key.len() < 32 {
        return Err("gateway_event_signing_key_invalid");
    }
    Ok(key)
}

pub fn load_account_registry(encoded: &str) -> Result<Vec<WompiAccountConfig>, &'static str> {
    let accounts: Vec<WompiAccountConfig> =
        serde_json::from_str(encoded).map_err(|_| "wompi_account_registry_invalid")?;
    if accounts.is_empty() {
        return Err("wompi_account_registry_empty");
    }
    let mut references = std::collections::HashSet::new();
    for account in &accounts {
        validate_account(account)?;
        if !references.insert(account.account_reference.as_str()) {
            return Err("wompi_account_reference_duplicate");
        }
    }
    Ok(accounts)
}
