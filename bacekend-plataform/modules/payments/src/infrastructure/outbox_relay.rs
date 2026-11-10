use std::{env, time::Duration};

use async_nats::jetstream::Context;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::hmac;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::infrastructure::persistence::{
    PendingOutboxEvent, claim_outbox_events, extend_outbox_lease, mark_outbox_failed,
    mark_outbox_published, reschedule_outbox_event,
};

const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);
const RECONCILIATION_REQUEST_EVENT: &str = "payment.reconciliation.requested";

enum PublishFailure {
    Retryable(&'static str),
    Permanent(&'static str),
}

pub struct GatewayRequestSigningConfig {
    key: Vec<u8>,
    key_id: String,
}

impl GatewayRequestSigningConfig {
    pub fn new(key: Vec<u8>, key_id: String) -> Result<Self, &'static str> {
        if key.len() < 32
            || key_id.is_empty()
            || key_id.len() > 64
            || !key_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
        {
            return Err("gateway_request_signing_configuration_invalid");
        }
        Ok(Self { key, key_id })
    }
}

pub async fn relay_batch(
    pool: &PgPool,
    jetstream: &Context,
    subject_prefix: &str,
) -> Result<usize, sqlx::Error> {
    let signing_config = load_signing_config_from_env();
    relay_batch_with_signing_config(pool, jetstream, subject_prefix, &signing_config).await
}

pub async fn relay_batch_with_signing_config(
    pool: &PgPool,
    jetstream: &Context,
    subject_prefix: &str,
    signing_config: &Result<GatewayRequestSigningConfig, &'static str>,
) -> Result<usize, sqlx::Error> {
    let events = claim_outbox_events(pool, 32).await?;
    let mut delivered = 0;
    for event in events {
        if !extend_outbox_lease(pool, event.event_id, event.lease_token).await? {
            tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="relay_outbox",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
            continue;
        }
        match publish_event(jetstream, &event, subject_prefix, signing_config).await {
            Ok(()) => {
                if mark_outbox_published(pool, event.event_id, event.lease_token).await? {
                    delivered += 1;
                    tracing::info!(event.name="outbox.event.published",component="payments",operation="relay_outbox",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                } else {
                    tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="acknowledge_outbox_event",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                }
            }
            Err(PublishFailure::Retryable(error_code)) => {
                if reschedule_outbox_event(pool, event.event_id, event.lease_token, error_code)
                    .await?
                {
                    tracing::error!(event.name="outbox.publish.failed",component="payments",operation="publish_outbox_event",dependency="nats_jetstream",error_code,error_category="dependency",retryable=true,event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                } else {
                    tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="reschedule_outbox_event",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                }
            }
            Err(PublishFailure::Permanent(error_code)) => {
                if mark_outbox_failed(pool, event.event_id, event.lease_token, error_code).await? {
                    tracing::error!(event.name="outbox.event.quarantined",component="payments",operation="quarantine_outbox_event",error_code,error_category="contract",retryable=false,event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                } else {
                    tracing::warn!(event.name="outbox.lease.lost",component="payments",operation="quarantine_outbox_event",event_id=%event.event_id,event_type=%event.event_type,correlation_id=%event.correlation_id);
                }
            }
        }
    }
    Ok(delivered)
}

async fn publish_event(
    jetstream: &Context,
    event: &PendingOutboxEvent,
    subject_prefix: &str,
    signing_config: &Result<GatewayRequestSigningConfig, &'static str>,
) -> Result<(), PublishFailure> {
    if event.event_type.is_empty()
        || !event.event_type.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'_'
        })
    {
        return Err(PublishFailure::Permanent("event_type_invalid"));
    }
    let subject = format!("{subject_prefix}.{}", event.event_type);
    let audience = event_audience(&event.event_type, &event.audience);
    let mut envelope = json!({
        "event_id": event.event_id,
        "event_type": event.event_type,
        "schema_version": event.schema_version,
        "aggregate_id": event.aggregate_id,
        "aggregate_version": event.aggregate_version,
        "audience": audience,
        "correlation_id": event.correlation_id,
        "traceparent": event.traceparent,
        "occurred_at": event.occurred_at.to_rfc3339(),
        "payload": event.payload
    });
    if event.event_type == RECONCILIATION_REQUEST_EVENT {
        let config = signing_config
            .as_ref()
            .map_err(|error_code| PublishFailure::Retryable(error_code))?;
        attach_producer_auth(&mut envelope, &config.key, &config.key_id)
            .map_err(PublishFailure::Retryable)?;
    }
    let payload = serde_json::to_vec(&envelope)
        .map_err(|_| PublishFailure::Permanent("event_serialization_failed"))?;
    let mut headers = async_nats::HeaderMap::new();
    headers.append("Nats-Msg-Id", event.event_id.to_string());
    headers.append("Payvia-Event-Type", event.event_type.clone());
    let ack = tokio::time::timeout(
        PUBLISH_TIMEOUT,
        jetstream.publish_with_headers(subject, headers, payload.into()),
    )
    .await
    .map_err(|_| PublishFailure::Retryable("jetstream_publish_timeout"))?
    .map_err(|_| PublishFailure::Retryable("jetstream_publish_unavailable"))?;
    tokio::time::timeout(PUBLISH_TIMEOUT, ack)
        .await
        .map_err(|_| PublishFailure::Retryable("jetstream_ack_timeout"))?
        .map(|_| ())
        .map_err(|_| PublishFailure::Retryable("jetstream_ack_unavailable"))
}

fn load_signing_config_from_env() -> Result<GatewayRequestSigningConfig, &'static str> {
    let encoded = env::var("PLATFORM_GATEWAY_REQUEST_SIGNING_KEY")
        .map_err(|_| "gateway_request_signing_key_missing")?;
    let key = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "gateway_request_signing_key_invalid")?;
    let key_id = env::var("PLATFORM_GATEWAY_REQUEST_SIGNING_KEY_ID")
        .map_err(|_| "gateway_request_signing_key_id_missing")?;
    GatewayRequestSigningConfig::new(key, key_id)
}

fn event_audience(event_type: &str, stored_audience: &str) -> &'static str {
    if event_type == RECONCILIATION_REQUEST_EVENT {
        "gateway"
    } else if stored_audience == "platform" {
        "platform_operator"
    } else {
        "tenant"
    }
}

fn attach_producer_auth(
    envelope: &mut Value,
    signing_key: &[u8],
    key_id: &str,
) -> Result<(), &'static str> {
    if signing_key.len() < 32
        || key_id.is_empty()
        || key_id.len() > 64
        || !key_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
    {
        return Err("gateway_request_signing_configuration_invalid");
    }
    if !envelope.is_object() {
        return Err("gateway_request_envelope_invalid");
    }
    if envelope.get("producer_auth").is_some() {
        return Err("gateway_request_envelope_already_signed");
    }
    let canonical = serde_json::to_vec(envelope).map_err(|_| "gateway_request_envelope_invalid")?;
    let signature = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, signing_key), &canonical);
    envelope
        .as_object_mut()
        .ok_or("gateway_request_envelope_invalid")?
        .insert(
            "producer_auth".to_owned(),
            json!({
                "key_id": key_id,
                "signature": hex_upper(signature.as_ref())
            }),
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciliation_requests_route_to_gateway_without_extending_database_audiences() {
        assert_eq!(
            event_audience(RECONCILIATION_REQUEST_EVENT, "platform"),
            "gateway"
        );
        assert_eq!(
            event_audience("payment_attempt.status_changed", "tenant"),
            "tenant"
        );
        assert_eq!(
            event_audience("tenant.suspended", "platform"),
            "platform_operator"
        );
    }

    #[test]
    fn reconciliation_requests_are_signed_for_the_gateway_audience() {
        let key = [23_u8; 32];
        let mut envelope = json!({
            "event_id": "2c02b42d-06c1-4b2c-9c51-f2e47fd2a8c8",
            "event_type": RECONCILIATION_REQUEST_EVENT,
            "schema_version": 1,
            "audience": "gateway",
            "payload": {"attempt_id":"f1bb8f4a-5c2e-4c8a-b0b5-0fb2f6d6f54c"}
        });
        attach_producer_auth(&mut envelope, &key, "platform-gateway-v1")
            .expect("valid platform signing configuration");

        let auth = envelope
            .as_object_mut()
            .expect("envelope object")
            .remove("producer_auth")
            .expect("producer authentication");
        assert_eq!(auth["key_id"], "platform-gateway-v1");
        let signature = decode_hex(auth["signature"].as_str().expect("signature"))
            .expect("valid signature encoding");
        let canonical = serde_json::to_vec(&envelope).expect("canonical event");
        hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, &key),
            &canonical,
            &signature,
        )
        .expect("gateway can verify the signed request");
    }

    #[test]
    fn signing_rejects_invalid_keys_key_ids_and_pre_signed_envelopes() {
        let mut envelope = json!({"event_id":"event-1"});
        assert_eq!(
            attach_producer_auth(&mut envelope, &[1_u8; 16], "platform-v1"),
            Err("gateway_request_signing_configuration_invalid")
        );
        assert_eq!(
            attach_producer_auth(&mut envelope, &[1_u8; 32], "Invalid Key"),
            Err("gateway_request_signing_configuration_invalid")
        );
        envelope["producer_auth"] = json!({"key_id":"attacker","signature":"00"});
        assert_eq!(
            attach_producer_auth(&mut envelope, &[1_u8; 32], "platform-v1"),
            Err("gateway_request_envelope_already_signed")
        );
    }

    fn decode_hex(value: &str) -> Option<Vec<u8>> {
        if value.len() != 64 {
            return None;
        }
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
            .collect()
    }
}
