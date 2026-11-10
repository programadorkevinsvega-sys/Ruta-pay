use super::*;
use serde_json::json;
use std::sync::Arc;

#[test]
fn verifies_signed_platform_reconciliation_requests_and_rejects_tampering() {
    let key = [31_u8; 32];
    let config = ReconciliationWorkerConfig {
        platform_request_key: Arc::from(key.to_vec()),
        platform_request_key_id: Arc::from("platform-gateway-v1"),
    };
    let mut envelope = json!({
        "event_id": "2c02b42d-06c1-4b2c-9c51-f2e47fd2a8c8",
        "event_type": "payment.reconciliation.requested",
        "schema_version": 1,
        "audience": "gateway",
        "correlation_id": "reconciliation-test",
        "payload": {"attempt_id":"f1bb8f4a-5c2e-4c8a-b0b5-0fb2f6d6f54c"}
    });
    let canonical = serde_json::to_vec(&envelope).expect("canonical event");
    let signature =
        hex_upper(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &key), &canonical).as_ref());
    envelope["producer_auth"] = json!({
        "key_id": "platform-gateway-v1",
        "signature": signature
    });

    verify_platform_event(&envelope, &config)
        .expect("platform-signed reconciliation request is authenticated");
    envelope["audience"] = json!("tenant");
    assert!(matches!(
        verify_platform_event(&envelope, &config),
        Err(RequestError::Unauthenticated)
    ));
}

#[test]
fn rejects_wrong_platform_key_id_and_short_verification_keys() {
    let key = [31_u8; 32];
    let config = ReconciliationWorkerConfig {
        platform_request_key: Arc::from(key.to_vec()),
        platform_request_key_id: Arc::from("expected-key"),
    };
    let envelope = json!({
        "event_id": "2c02b42d-06c1-4b2c-9c51-f2e47fd2a8c8",
        "producer_auth": {"key_id":"other-key","signature":"00".repeat(32)}
    });
    assert!(matches!(
        verify_platform_event(&envelope, &config),
        Err(RequestError::Unauthenticated)
    ));

    let short_key = [31_u8; 16];
    let short_config = ReconciliationWorkerConfig {
        platform_request_key: Arc::from(short_key.to_vec()),
        platform_request_key_id: Arc::from("expected-key"),
    };
    let mut short_envelope = json!({"event_id":"short-key-event"});
    let canonical = serde_json::to_vec(&short_envelope).expect("canonical event");
    let signature =
        hex_upper(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &short_key), &canonical).as_ref());
    short_envelope["producer_auth"] = json!({
        "key_id": "expected-key",
        "signature": signature
    });
    assert!(matches!(
        verify_platform_event(&short_envelope, &short_config),
        Err(RequestError::Unauthenticated)
    ));
}
