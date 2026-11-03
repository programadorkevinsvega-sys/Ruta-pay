use async_nats::jetstream::{
    self,
    consumer::{AckPolicy, DeliverPolicy},
};
use platform_payments::{
    application::payment_service::{CreatePaymentIntentCommand, PaymentService},
    infrastructure::outbox_relay,
};
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use redis::aio::ConnectionManager;
use serde_json::Value;
use sqlx::Row;
use std::time::Duration;
use tokio_stream::StreamExt;
use uuid::Uuid;

use super::database::TestDatabase;

pub async fn verify() {
    let database = TestDatabase::new().await;
    let tenant_id = Uuid::new_v4();
    let service = PaymentService::new(database.runtime_pool.clone());
    let (intent, replayed) = service
        .create_payment_intent(CreatePaymentIntentCommand {
            tenant_id,
            actor_key_id: Uuid::new_v4(),
            environment: "test".to_owned(),
            amount_minor: 1299,
            currency: "COP".to_owned(),
            payment_method_type: "card".to_owned(),
            merchant_reference: Some("order-outbox-001".to_owned()),
            idempotency_key: "phase4-outbox-publish-001".to_owned(),
            correlation_id: "phase4_payments_jetstream_acceptance".to_owned(),
        })
        .await
        .expect("Payment Intent and Outbox event commit atomically");
    assert!(!replayed);

    let outbox = sqlx::query(
        "SELECT event_id, occurred_at FROM payments.outbox_events WHERE aggregate_id=$1",
    )
    .bind(intent.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("Payment Intent Outbox event is persisted");
    let event_id: Uuid = outbox.try_get("event_id").expect("event ID is persisted");
    let occurred_at: chrono::DateTime<chrono::Utc> = outbox
        .try_get("occurred_at")
        .expect("original event timestamp is persisted");

    let nats_url = std::env::var("PLATFORM_NATS_URL").expect("CI provides JetStream URL");
    let nats_user = std::env::var("NATS_USER").expect("CI provides JetStream user");
    let nats_password = std::env::var("NATS_PASSWORD").expect("CI provides JetStream password");
    let client = async_nats::ConnectOptions::new()
        .user_and_password(nats_user, nats_password)
        .connect(nats_url)
        .await
        .expect("authenticated JetStream connection succeeds");
    let jetstream = jetstream::new(client);
    let namespace = format!("e2e_phase4_{}", Uuid::new_v4().simple());
    let topology =
        EventTopology::from_namespace(Some(&namespace)).expect("isolated event namespace is valid");
    let stream = topology
        .ensure_stream(&jetstream)
        .await
        .expect("isolated payment event stream is created");

    let redis_url = std::env::var("PLATFORM_REDIS_URL").expect("CI provides Redis URL");
    let redis_client = redis::Client::open(redis_url).expect("Redis URL parses");
    let mut redis = ConnectionManager::new(redis_client)
        .await
        .expect("authenticated Redis connection succeeds");

    let published = outbox_relay::relay_batch(
        &database.runtime_pool,
        &jetstream,
        &topology.subject_prefix,
    )
    .await
    .expect("payments Outbox relay publishes the event");
    assert_eq!(published, 1, "JetStream publish ACK marks one event delivered");

    let consumer_name = format!("PHASE4_ACCEPTANCE_{}", Uuid::new_v4().simple());
    let consumer = stream
        .get_or_create_consumer(
            &consumer_name,
            jetstream::consumer::pull::Config {
                durable_name: Some(consumer_name),
                filter_subject: format!("{}.>", topology.subject_prefix),
                deliver_policy: DeliverPolicy::All,
                ack_policy: AckPolicy::Explicit,
                ..Default::default()
            },
        )
        .await
        .expect("isolated durable payment consumer is created");
    let mut messages = consumer
        .messages()
        .await
        .expect("durable payment consumer stream opens");
    let message = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("JetStream delivers the payment event before timeout")
        .expect("durable consumer remains open")
        .expect("JetStream message is valid");
    let event_payload = message.payload.clone();
    let envelope: Value =
        serde_json::from_slice(&event_payload).expect("published event envelope is JSON");

    assert_eq!(envelope["event_id"], event_id.to_string());
    assert_eq!(envelope["event_type"], "payment_intent.created");
    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["aggregate_id"], intent.id.to_string());
    assert_eq!(envelope["audience"], "tenant");
    assert_eq!(
        envelope["occurred_at"],
        occurred_at.to_rfc3339(),
        "relay preserves the business event timestamp"
    );
    assert_eq!(envelope["correlation_id"], "phase4_payments_jetstream_acceptance");
    assert_eq!(envelope["payload"]["tenant_id"], tenant_id.to_string());
    assert_eq!(envelope["payload"]["payment_intent_id"], intent.id.to_string());
    assert!(
        envelope["payload"].get("idempotency_key").is_none(),
        "commercial idempotency keys are not published"
    );

    let inserted = platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
        &database.runtime_pool,
        &mut redis,
        &event_payload,
    )
    .await
    .expect("realtime consumer persists the verified payment event and signals Redis");
    assert!(inserted, "first delivery enters the realtime journal");
    message
        .ack()
        .await
        .expect("consumer ACK follows successful journal persistence");

    let duplicate_inserted =
        platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
            &database.runtime_pool,
            &mut redis,
            &event_payload,
        )
        .await
        .expect("redelivered event is safely deduplicated");
    assert!(
        !duplicate_inserted,
        "same event ID never increments the durable realtime journal twice"
    );

    let tenant_events = platform_realtime::infrastructure::persistence::replay(
        &database.runtime_pool,
        "tenant",
        Some(tenant_id),
        0,
        10,
    )
    .await
    .expect("tenant can recover its authorized payment event by cursor");
    assert_eq!(tenant_events.len(), 1);
    assert_eq!(tenant_events[0].event_id, event_id);
    assert_eq!(tenant_events[0].cursor, 1);

    let other_tenant_events = platform_realtime::infrastructure::persistence::replay(
        &database.runtime_pool,
        "tenant",
        Some(Uuid::new_v4()),
        0,
        10,
    )
    .await
    .expect("another tenant receives an isolated empty cursor");
    assert!(
        other_tenant_events.is_empty(),
        "realtime replay cannot leak another tenant's payment event"
    );

    sqlx::query(
        "UPDATE payments.outbox_events SET published_at=NULL,available_at=now(),lease_token=NULL,lease_until=NULL WHERE event_id=$1",
    )
    .bind(event_id)
    .execute(&database.migration_pool)
    .await
    .expect("simulate relay crash after JetStream ACK but before Outbox acknowledgement");

    let replayed_publish = outbox_relay::relay_batch(
        &database.runtime_pool,
        &jetstream,
        &topology.subject_prefix,
    )
    .await
    .expect("Outbox retry republishes with the stable Nats-Msg-Id");
    assert_eq!(replayed_publish, 1, "retry is acknowledged and Outbox closes");

    let final_state = sqlx::query(
        "SELECT attempts, published_at, lease_token, last_error_code FROM payments.outbox_events WHERE event_id=$1",
    )
    .bind(event_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("Outbox delivery state is queryable");
    let attempts: i32 = final_state.try_get("attempts").expect("attempt count exists");
    let published_at: Option<chrono::DateTime<chrono::Utc>> = final_state
        .try_get("published_at")
        .expect("publish timestamp exists");
    let lease_token: Option<Uuid> = final_state.try_get("lease_token").expect("lease is readable");
    let last_error: Option<String> = final_state
        .try_get("last_error_code")
        .expect("last error is readable");
    assert_eq!(attempts, 2, "the crash-window retry is recorded");
    assert!(published_at.is_some(), "ACKed event is marked published");
    assert!(lease_token.is_none(), "published event releases its lease");
    assert!(last_error.is_none(), "successful retry clears the prior error");

    let stream_info = stream.info().await.expect("stream state is queryable");
    assert_eq!(
        stream_info.state.messages, 1,
        "JetStream deduplicates the repeated publish by event ID"
    );
    let pending = outbox_relay::relay_batch(
        &database.runtime_pool,
        &jetstream,
        &topology.subject_prefix,
    )
    .await
    .expect("empty Outbox batch is harmless");
    assert_eq!(pending, 0, "published events are not claimed again");

    drop(message);
    drop(messages);
    drop(consumer);
    delete_e2e_stream(&jetstream, &namespace)
        .await
        .expect("only the isolated phase four JetStream stream is deleted");
    database.finish().await;
}
