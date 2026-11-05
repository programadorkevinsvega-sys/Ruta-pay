use ring::hmac;
use serde_json::Value;

use super::PaymentServiceError;

pub(super) fn verify_signed_event(
    payload: &[u8],
    signing_key: &[u8],
    expected_key_id: &str,
) -> Result<Value, PaymentServiceError> {
    if signing_key.len() < 32 || expected_key_id.is_empty() || expected_key_id.len() > 64 {
        return Err(PaymentServiceError::ProviderEventInvalid);
    }
    let envelope: Value = serde_json::from_slice(payload)?;
    let key_id = envelope
        .get("key_id")
        .and_then(Value::as_str)
        .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    if key_id != expected_key_id {
        return Err(PaymentServiceError::ProviderEventInvalid);
    }
    let signature = envelope
        .get("signature")
        .and_then(Value::as_str)
        .and_then(decode_hex)
        .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let event = envelope
        .get("event")
        .cloned()
        .ok_or(PaymentServiceError::ProviderEventInvalid)?;
    let canonical = serde_json::to_vec(&event)?;
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, signing_key),
        &canonical,
        &signature,
    )
    .map_err(|_| PaymentServiceError::ProviderEventInvalid)?;
    Ok(event)
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


#[cfg(test)]
fn encode_hex(value: &[u8]) -> String {
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
    use serde_json::json;

    const KEY_ID: &str = "gateway-events-v1";

    fn signed_payload(event: Value, key: &[u8]) -> Vec<u8> {
        let canonical = serde_json::to_vec(&event).expect("canonical event");
        let signature = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), &canonical);
        serde_json::to_vec(&json!({
            "key_id": KEY_ID,
            "event": event,
            "signature": encode_hex(signature.as_ref())
        }))
        .expect("signed envelope")
    }

    #[test]
    fn accepts_a_valid_event_signature_and_key_id() {
        let key = [9_u8; 32];
        let event = json!({"event_id":"b6f0c2a9-0cdb-4ddc-8b08-a1e7a6dcbf52","status":"succeeded"});
        let signed = signed_payload(event.clone(), &key);

        assert_eq!(
            verify_signed_event(&signed, &key, KEY_ID).expect("valid signature"),
            event
        );
    }

    #[test]
    fn rejects_tampering_wrong_key_id_and_short_keys() {
        let key = [9_u8; 32];
        let event = json!({"event_id":"b6f0c2a9-0cdb-4ddc-8b08-a1e7a6dcbf52","status":"succeeded"});
        let mut signed = signed_payload(event, &key);
        signed = String::from_utf8(signed)
            .expect("utf8")
            .replace("succeeded", "failed")
            .into_bytes();
        assert!(matches!(
            verify_signed_event(&signed, &key, KEY_ID),
            Err(PaymentServiceError::ProviderEventInvalid)
        ));

        let signed = signed_payload(json!({"status":"succeeded"}), &key);
        assert!(matches!(
            verify_signed_event(&signed, &key, "another-key"),
            Err(PaymentServiceError::ProviderEventInvalid)
        ));
        assert!(matches!(
            verify_signed_event(&signed, &[9_u8; 16], KEY_ID),
            Err(PaymentServiceError::ProviderEventInvalid)
        ));
    }
}
