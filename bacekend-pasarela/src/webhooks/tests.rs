use super::*;

#[test]
fn account_registry_rejects_cross_environment_event_secrets() {
    let account = WompiAccountConfig {
        account_reference: "merchant_01".to_owned(),
        tenant_id: Uuid::new_v4(),
        environment: "test".to_owned(),
        event_secret: "prod_events_not_for_test".to_owned(),
        private_key: "prv_test_account_private_key".to_owned(),
    };
    assert_eq!(
        validate_account(&account),
        Err("wompi_event_secret_environment_mismatch")
    );
}

#[test]
fn account_registry_rejects_invalid_and_duplicate_references() {
    let first = WompiAccountConfig {
        account_reference: "merchant_01".to_owned(),
        tenant_id: Uuid::new_v4(),
        environment: "test".to_owned(),
        event_secret: "test_events_test_secret_long_enough".to_owned(),
        private_key: "prv_test_private_key_long_enough".to_owned(),
    };
    let second = WompiAccountConfig {
        account_reference: "merchant_01".to_owned(),
        tenant_id: Uuid::new_v4(),
        environment: "test".to_owned(),
        event_secret: "test_events_second_secret_long_enough".to_owned(),
        private_key: "prv_test_second_private_key_long".to_owned(),
    };
    let parsed = serde_json::to_string(&vec![
        serde_json::json!({
            "account_reference":first.account_reference,
            "tenant_id":first.tenant_id,
            "environment":first.environment,
            "event_secret":first.event_secret,
            "private_key":first.private_key
        }),
        serde_json::json!({
            "account_reference":second.account_reference,
            "tenant_id":second.tenant_id,
            "environment":second.environment,
            "event_secret":second.event_secret,
            "private_key":second.private_key
        }),
    ])
    .expect("serialize");
    assert!(matches!(
        load_account_registry(&parsed),
        Err("wompi_account_reference_duplicate")
    ));
}

#[test]
fn signing_key_requires_a_minimum_256_bits() {
    let short = STANDARD.encode([7_u8; 16]);
    assert_eq!(
        load_event_signing_key(&short),
        Err("gateway_event_signing_key_invalid")
    );
    let valid = STANDARD.encode([7_u8; 32]);
    assert_eq!(load_event_signing_key(&valid).expect("valid").len(), 32);
}
