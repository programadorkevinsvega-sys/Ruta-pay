use async_nats::jetstream::{self, Context};
use platform_payments::infrastructure::{
    outbox_relay,
    persistence::{claim_outbox_events, extend_outbox_lease, mark_outbox_published},
};
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::database::TestDatabase;

async fn insert_event(pool: &PgPool, event_type: &str) -> Uuid {
    let event_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payments.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,1,$3,1,'tenant',$4,$5)",
    )
    .bind(event_id)
    .bind(Uuid::new_v4())
    .bind(event_type)
    .bind(json!({"tenant_id": Uuid::new_v4().to_string()}))
    .bind("phase4_outbox_resilience")
    .execute(pool)
    .await
    .expect("outbox test event is persisted in PostgreSQL");
    event_id
}

async fn connect_jetstream() -> Context {
    let nats_url = std::env::var("PLATFORM_NATS_URL").expect("CI provides JetStream URL");
    let nats_user = std::env::var("NATS_USER").expect("CI provides JetStream user");
    let nats_password = std::env::var("NATS_PASSWORD").expect("CI provides JetStream password");
    let client = async_nats::ConnectOptions::new()
        .user_and_password(nats_user, nats_password)
        .connect(nats_url)
        .await
        .expect("authenticated JetStream connection succeeds");
    jetstream::new(client)
}

fn topology() -> (String, EventTopology) {
    let namespace = format!("e2e_outbox_{}", Uuid::new_v4().simple());
    let topology = EventTopology::from_namespace(Some(&namespace))
        .expect("isolated outbox namespace is valid");
    (namespace, topology)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbox_lease_expiry_is_reclaimed_and_stale_owner_is_rejected() {
    let database = TestDatabase::new().await;
    let event_id = insert_event(&database.runtime_pool, "payment_intent.created").await;
    let first = claim_outbox_events(&database.runtime_pool, 1)
        .await
        .expect("first relay claims the event");
    assert_eq!(first.len(), 1);
    let stale_token = first[0].lease_token;

    sqlx::query(
        "UPDATE payments.outbox_events SET lease_until=now()-interval '1 second' WHERE event_id=$1",
    )
    .bind(event_id)
    .execute(&database.migration_pool)
    .await
    .expect("test expires only the isolated event lease");

    assert!(
        !extend_outbox_lease(&database.runtime_pool, event_id, stale_token)
            .await
            .expect("expired owner lease renewal is checked"),
        "an expired lease cannot be renewed without a new claim"
    );

    let recovered = claim_outbox_events(&database.runtime_pool, 1)
        .await
        .expect("another relay recovers the expired event");
    assert_eq!(recovered.len(), 1);
    let current_token = recovered[0].lease_token;
    assert_ne!(stale_token, current_token);

    assert!(
        !extend_outbox_lease(&database.runtime_pool, event_id, stale_token)
            .await
            .expect("stale owner lease renewal is checked"),
        "a relay that lost its token cannot renew the lease"
    );
    assert!(
        !mark_outbox_published(&database.runtime_pool, event_id, stale_token)
            .await
            .expect("stale owner acknowledgement is checked"),
        "a relay that lost its token cannot mark the event published"
    );
    assert!(
        extend_outbox_lease(&database.runtime_pool, event_id, current_token)
            .await
            .expect("current lease renewal is checked"),
        "the recovered owner can extend its lease"
    );
    database.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbox_retries_after_stream_becomes_available_with_jittered_backoff() {
    let database = TestDatabase::new().await;
    let event_id = insert_event(&database.runtime_pool, "payment_intent.created").await;
    let jetstream = connect_jetstream().await;
    let (namespace, topology) = topology();

    let first =
        outbox_relay::relay_batch(&database.runtime_pool, &jetstream, &topology.subject_prefix)
            .await
            .expect("temporary JetStream rejection is rescheduled");
    assert_eq!(first, 0, "a rejected publish is not acknowledged");

    let deferred = sqlx::query(
        "SELECT attempts,available_at,published_at,lease_token,last_error_code FROM payments.outbox_events WHERE event_id=$1",
    )
    .bind(event_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("rescheduled event is queryable");
    let attempts: i32 = sqlx::Row::try_get(&deferred, "attempts").expect("attempt count exists");
    let available_at: chrono::DateTime<chrono::Utc> =
        sqlx::Row::try_get(&deferred, "available_at").expect("retry time exists");
    let published_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::Row::try_get(&deferred, "published_at").expect("publish time exists");
    let lease_token: Option<Uuid> =
        sqlx::Row::try_get(&deferred, "lease_token").expect("lease is readable");
    let last_error: Option<String> =
        sqlx::Row::try_get(&deferred, "last_error_code").expect("error code is readable");
    assert_eq!(attempts, 1);
    assert!(available_at > chrono::Utc::now());
    assert!(published_at.is_none());
    assert!(lease_token.is_none());
    assert!(last_error.is_some());

    topology
        .ensure_stream(&jetstream)
        .await
        .expect("JetStream stream is restored");
    sqlx::query("UPDATE payments.outbox_events SET available_at=now() WHERE event_id=$1")
        .bind(event_id)
        .execute(&database.migration_pool)
        .await
        .expect("test makes the recovered event immediately eligible");
    let recovered =
        outbox_relay::relay_batch(&database.runtime_pool, &jetstream, &topology.subject_prefix)
            .await
            .expect("relay recovers after JetStream becomes available");
    assert_eq!(recovered, 1);

    delete_e2e_stream(&jetstream, &namespace)
        .await
        .expect("only the isolated test stream is deleted");
    database.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_relays_publish_all_claimed_events_without_losing_rows() {
    let database = TestDatabase::new().await;
    for _ in 0..40 {
        insert_event(&database.runtime_pool, "payment_intent.created").await;
    }
    let jetstream = connect_jetstream().await;
    let (namespace, topology) = topology();
    let mut stream = topology
        .ensure_stream(&jetstream)
        .await
        .expect("isolated stream is created");

    let (first, second) = tokio::join!(
        outbox_relay::relay_batch(&database.runtime_pool, &jetstream, &topology.subject_prefix),
        outbox_relay::relay_batch(&database.runtime_pool, &jetstream, &topology.subject_prefix)
    );
    let delivered = first.expect("first relay completes") + second.expect("second relay completes");
    assert_eq!(
        delivered, 40,
        "both relays publish the full claimed backlog"
    );

    let published: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.outbox_events WHERE published_at IS NOT NULL",
    )
    .fetch_one(&database.migration_pool)
    .await
    .expect("published event count is readable");
    assert_eq!(published, 40);
    let info = stream
        .info()
        .await
        .expect("JetStream stream state is readable");
    assert_eq!(
        info.state.messages, 40,
        "each event has a stable unique message ID"
    );

    delete_e2e_stream(&jetstream, &namespace)
        .await
        .expect("only the isolated test stream is deleted");
    database.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn permanent_outbox_errors_are_quarantined_without_retrying_forever() {
    let database = TestDatabase::new().await;
    let event_id = insert_event(&database.runtime_pool, "Invalid Type").await;
    let jetstream = connect_jetstream().await;
    let (_, topology) = topology();

    let delivered =
        outbox_relay::relay_batch(&database.runtime_pool, &jetstream, &topology.subject_prefix)
            .await
            .expect("invalid event contract is quarantined without a publish attempt");
    assert_eq!(delivered, 0);

    let failed = sqlx::query(
        "SELECT attempts,failed_at,lease_token,last_error_code FROM payments.outbox_events WHERE event_id=$1",
    )
    .bind(event_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("quarantined event remains inspectable");
    let attempts: i32 = sqlx::Row::try_get(&failed, "attempts").expect("attempt count exists");
    let failed_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::Row::try_get(&failed, "failed_at").expect("failure timestamp exists");
    let lease_token: Option<Uuid> =
        sqlx::Row::try_get(&failed, "lease_token").expect("lease is readable");
    let error_code: Option<String> =
        sqlx::Row::try_get(&failed, "last_error_code").expect("error code is readable");
    assert_eq!(attempts, 1);
    assert!(failed_at.is_some());
    assert!(lease_token.is_none());
    assert_eq!(error_code.as_deref(), Some("event_type_invalid"));

    let repeated =
        outbox_relay::relay_batch(&database.runtime_pool, &jetstream, &topology.subject_prefix)
            .await
            .expect("quarantined rows are excluded from automatic claims");
    assert_eq!(repeated, 0);
    let final_attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM payments.outbox_events WHERE event_id=$1")
            .bind(event_id)
            .fetch_one(&database.migration_pool)
            .await
            .expect("quarantined attempt count is readable");
    assert_eq!(
        final_attempts, 1,
        "permanent errors do not retry indefinitely"
    );
    database.finish().await;
}


#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lifecycle_outbox_event_types_are_acknowledged_by_real_jetstream() {
    let database = TestDatabase::new().await;
    let event_types = [
        "payment_attempt.created",
        "payment_intent.status_changed",
        "payment_attempt.status_changed",
        "payment.reconciliation.requested",
    ];
    for event_type in event_types {
        insert_event(&database.runtime_pool, event_type).await;
    }
    let jetstream = connect_jetstream().await;
    let (namespace, topology) = topology();
    let stream = topology
        .ensure_stream(&jetstream)
        .await
        .expect("stream accepts the lifecycle event subject namespace");

    let delivered =
        outbox_relay::relay_batch(&database.runtime_pool, &jetstream, &topology.subject_prefix)
            .await
            .expect("relay publishes all lifecycle events to JetStream");
    assert_eq!(delivered, event_types.len());

    let published: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.outbox_events WHERE event_type = ANY($1) AND published_at IS NOT NULL",
    )
    .bind(event_types.iter().map(|event_type| (*event_type).to_owned()).collect::<Vec<_>>())
    .fetch_one(&database.migration_pool)
    .await
    .expect("lifecycle publish ACKs are persisted");
    assert_eq!(published, event_types.len() as i64);

    let info = stream.info().await.expect("stream info is readable");
    assert_eq!(info.state.messages, event_types.len() as u64);

    delete_e2e_stream(&jetstream, &namespace)
        .await
        .expect("only this isolated lifecycle stream is deleted");
    database.finish().await;
}
