use chrono::Utc;
use platform_payments::application::payment_service::{
    GatewayEventDisposition, PaymentService, PaymentServiceError,
};
use ring::hmac;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::super::database::TestDatabase;
use super::support::{attempt_command, intent_command, status_command};

const KEY_ID: &str = "gateway-events-v1";
const SIGNING_KEY: [u8; 32] = [19_u8; 32];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_provider_events_apply_once_and_terminal_conflicts_require_reconciliation() {
    let database = TestDatabase::new().await;
    let service = PaymentService::new(database.runtime_pool.clone());
    let tenant_id = Uuid::new_v4();
    let actor_key_id = Uuid::new_v4();
    let (intent, replayed) = service
        .create_payment_intent(intent_command(tenant_id, actor_key_id))
        .await
        .expect("intent is created");
    assert!(!replayed);
    let (attempt, replayed) = service
        .create_payment_attempt(attempt_command(
            tenant_id,
            actor_key_id,
            intent.id,
            "provider-event-attempt-001",
        ))
        .await
        .expect("attempt is created");
    assert!(!replayed);

    service
        .update_payment_intent_status(status_command(
            tenant_id,
            actor_key_id,
            intent.id,
            attempt.id,
            "processing",
            1,
        ))
        .await
        .expect("the operation is submitted before provider evidence is accepted");

    let succeeded = provider_event(
        tenant_id,
        attempt.id,
        "provider-event-approved-001",
        "succeeded",
        Utc::now().timestamp_millis() + 10_000,
    );
    let signed = sign_event(succeeded.clone());
    let applied = service
        .ingest_signed_provider_event(&signed, &SIGNING_KEY, KEY_ID)
        .await
        .expect("gateway-signed provider event applies");
    assert_eq!(applied, GatewayEventDisposition::Applied);

    let replay = service
        .ingest_signed_provider_event(&signed, &SIGNING_KEY, KEY_ID)
        .await
        .expect("same provider event is idempotent");
    assert_eq!(replay, GatewayEventDisposition::Duplicate);

    let statuses = sqlx::query(
        "SELECT i.status AS intent_status,a.status AS attempt_status FROM payments.payment_intents i JOIN payments.payment_attempts a ON a.payment_intent_id=i.id WHERE i.id=$1 AND a.id=$2",
    )
    .bind(intent.id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("payment states are queryable");
    assert_eq!(statuses.try_get::<String, _>("intent_status").expect("intent status"), "succeeded");
    assert_eq!(statuses.try_get::<String, _>("attempt_status").expect("attempt status"), "succeeded");

    let transaction_status: String = sqlx::query_scalar(
        "SELECT status FROM payments.payment_transactions WHERE tenant_id=$1 AND payment_attempt_id=$2",
    )
    .bind(tenant_id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("provider transaction is persisted");
    assert_eq!(transaction_status, "succeeded");

    let inbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.provider_event_inbox WHERE tenant_id=$1 AND payment_attempt_id=$2",
    )
    .bind(tenant_id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("inbox count is queryable");
    assert_eq!(inbox_count, 1);

    let delivery_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.provider_event_deliveries WHERE tenant_id=$1 AND event_id=$2",
    )
    .bind(tenant_id)
    .bind(succeeded["event_id"].as_str().expect("event id"))
    .fetch_one(&database.migration_pool)
    .await
    .expect("delivery ledger count is queryable");
    assert_eq!(delivery_count, 2, "first delivery and replay are both retained");

    let mut conflicting = provider_event(
        tenant_id,
        attempt.id,
        "provider-event-declined-after-approval",
        "failed",
        Utc::now().timestamp_millis() + 20_000,
    );
    conflicting["provider_transaction_reference"] =
        succeeded["provider_transaction_reference"].clone();
    let conflict_result = service
        .ingest_signed_provider_event(
            &sign_event(conflicting),
            &SIGNING_KEY,
            KEY_ID,
        )
        .await
        .expect("a contradictory terminal event is retained for reconciliation");
    assert_eq!(
        conflict_result,
        GatewayEventDisposition::ReconciliationRequired
    );

    let case = sqlx::query(
        "SELECT reason,status FROM payments.payment_reconciliation_cases WHERE tenant_id=$1 AND payment_attempt_id=$2",
    )
    .bind(tenant_id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("terminal conflict creates a durable reconciliation case");
    assert_eq!(case.try_get::<String, _>("reason").expect("case reason"), "terminal_conflict");
    assert_eq!(case.try_get::<String, _>("status").expect("case status"), "pending");

    let final_status: String = sqlx::query_scalar(
        "SELECT status FROM payments.payment_attempts WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant_id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("final attempt status is queryable");
    assert_eq!(final_status, "succeeded", "terminal state is never overwritten");

    let mut invalid = provider_event(
        tenant_id,
        attempt.id,
        "provider-event-tampered",
        "failed",
        Utc::now().timestamp_millis() + 30_000,
    );
    let mut tampered = serde_json::from_slice::<Value>(
        &sign_event(invalid.take()),
    )
    .expect("signed envelope is JSON");
    tampered["signature"] = Value::String("00".repeat(32));
    let rejected = service
        .ingest_signed_provider_event(
            &serde_json::to_vec(&tampered).expect("tampered envelope serializes"),
            &SIGNING_KEY,
            KEY_ID,
        )
        .await;
    assert!(matches!(
        rejected,
        Err(PaymentServiceError::ProviderEventInvalid)
    ));

    database.finish().await;
}

fn provider_event(
    tenant_id: Uuid,
    attempt_id: Uuid,
    provider_event_id: &str,
    status: &str,
    occurred_at_unix_millis: i64,
) -> Value {
    let raw_digest = Sha256::digest(format!("{provider_event_id}:raw-callback").as_bytes());
    json!({
        "schema_version": 1,
        "event_id": Uuid::new_v4(),
        "provider_code": "wompi",
        "provider_account_reference": "account_01",
        "provider_event_id": provider_event_id,
        "tenant_id": tenant_id,
        "environment": "test",
        "payment_attempt_id": attempt_id,
        "provider_transaction_reference": "wompi-transaction-001",
        "status": status,
        "amount_minor": 1299,
        "currency": "COP",
        "occurred_at_unix_millis": occurred_at_unix_millis,
        "correlation_id": format!("correlation-{provider_event_id}"),
        "payload_sha256": hex_upper(&raw_digest)
    })
}

fn sign_event(event: Value) -> Vec<u8> {
    let canonical = serde_json::to_vec(&event).expect("event serializes");
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, &SIGNING_KEY),
        &canonical,
    );
    serde_json::to_vec(&json!({
        "key_id": KEY_ID,
        "event": event,
        "signature": hex_upper(signature.as_ref())
    }))
    .expect("signed gateway event serializes")
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
