use super::*;

const EVENT_SECRET: &str = "prod_events_OcHnIzeBl5socpwByQ4hA52Em3USQ93Z";

#[test]
fn matches_the_official_wompi_sha256_checksum_vector() {
    let input = concat!(
        "1234-1610641025-49201",
        "APPROVED",
        "4490000",
        "1530291411",
        "prod_events_OcHnIzeBl5socpwByQ4hA52Em3USQ93Z"
    );
    assert_eq!(
        super::super::event::sha256_hex_upper(input.as_bytes()),
        "3476DDA50F64CD7CBD160689640506FEBEA93239BC524FC0469B2C68A3CC8BD0"
    );
}

#[test]
fn verifies_dynamic_properties_and_normalizes_a_transaction_event() {
    let attempt_id = Uuid::new_v4();
    let raw = signed_event(attempt_id, "APPROVED", "prod", EVENT_SECRET);
    let verified = verify_event(
        raw.as_bytes(),
        Some(&checksum_from_body(&raw)),
        EVENT_SECRET,
        WompiEnvironment::Live,
    )
    .expect("official event signature must verify");

    assert_eq!(verified.attempt_id, attempt_id);
    assert_eq!(verified.status, WompiPaymentStatus::Succeeded);
    assert_eq!(verified.amount_minor, 15_900);
    assert_eq!(verified.currency, "COP");
    assert_eq!(
        verified.provider_transaction_reference,
        "wompi-transaction-001"
    );
    assert_eq!(verified.occurred_at.timestamp(), 1_730_000_000);
    let replay = verify_event(
        raw.as_bytes(),
        None,
        EVENT_SECRET,
        WompiEnvironment::Live,
    )
    .expect("same signed delivery remains verifiable for idempotent handling");
    assert_eq!(verified.event_id, replay.event_id);
    assert_eq!(verified.provider_event_id, replay.provider_event_id);
}

#[test]
fn rejects_header_checksum_mismatch_even_when_body_checksum_is_valid() {
    let raw = signed_event(Uuid::new_v4(), "PENDING", "prod", EVENT_SECRET);
    assert_eq!(
        verify_event(
            raw.as_bytes(),
            Some(&"A".repeat(64)),
            EVENT_SECRET,
            WompiEnvironment::Live
        ),
        Err(WompiEventError::InvalidSignature)
    );
}

#[test]
fn rejects_wrong_account_environment_and_unknown_status() {
    let raw = signed_event(Uuid::new_v4(), "PENDING", "prod", EVENT_SECRET);
    assert_eq!(
        verify_event(
            raw.as_bytes(),
            None,
            EVENT_SECRET,
            WompiEnvironment::Test
        ),
        Err(WompiEventError::EnvironmentMismatch)
    );

    let unknown = signed_event(Uuid::new_v4(), "IN_REVIEW", "prod", EVENT_SECRET);
    assert_eq!(
        verify_event(
            unknown.as_bytes(),
            None,
            EVENT_SECRET,
            WompiEnvironment::Live
        ),
        Err(WompiEventError::UnsupportedStatus)
    );
}

#[test]
fn rejects_missing_property_invalid_reference_and_corrupted_body() {
    let mut raw: Value = serde_json::from_str(&signed_event(
        Uuid::new_v4(),
        "PENDING",
        "prod",
        EVENT_SECRET,
    ))
    .expect("valid json");
    raw["signature"]["properties"][0] = Value::String("transaction.not_present".to_owned());
    let raw = serde_json::to_string(&raw).expect("serialize");
    assert_eq!(
        verify_event(&raw.as_bytes(), None, EVENT_SECRET, WompiEnvironment::Live),
        Err(WompiEventError::InvalidSignature)
    );

    let raw = signed_event_with_reference("not-an-attempt", "PENDING", "prod", EVENT_SECRET);
    assert_eq!(
        verify_event(&raw.as_bytes(), None, EVENT_SECRET, WompiEnvironment::Live),
        Err(WompiEventError::InvalidReference)
    );

    let raw = signed_event(Uuid::new_v4(), "PENDING", "prod", EVENT_SECRET);
    let corrupted = raw.replace("15900", "15901");
    assert_eq!(
        verify_event(
            corrupted.as_bytes(),
            None,
            EVENT_SECRET,
            WompiEnvironment::Live
        ),
        Err(WompiEventError::InvalidSignature)
    );
}

fn checksum_from_body(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .expect("json")
        .get("signature")
        .and_then(|value| value.get("checksum"))
        .and_then(Value::as_str)
        .expect("checksum")
        .to_owned()
}

fn signed_event(
    attempt_id: Uuid,
    status: &str,
    environment: &str,
    secret: &str,
) -> String {
    signed_event_with_reference(
        &format!("rpay_{attempt_id}"),
        status,
        environment,
        secret,
    )
}

fn signed_event_with_reference(
    reference: &str,
    status: &str,
    environment: &str,
    secret: &str,
) -> String {
    let timestamp = 1_730_000_000_i64;
    let transaction_id = "wompi-transaction-001";
    let amount = 15_900_i64;
    let signature_input = format!("{transaction_id}{status}{amount}{timestamp}{secret}");
    let checksum = super::super::event::sha256_hex_upper(signature_input.as_bytes());
    serde_json::json!({
        "event":"transaction.updated",
        "data":{"transaction":{
            "id":transaction_id,
            "reference":reference,
            "status":status,
            "amount_in_cents":amount,
            "currency":"COP"
        }},
        "environment":environment,
        "signature":{
            "properties":[
                "transaction.id",
                "transaction.status",
                "transaction.amount_in_cents"
            ],
            "checksum":checksum
        },
        "timestamp":timestamp,
        "sent_at":"2024-11-13T22:13:20.000Z"
    }).to_string()
}
