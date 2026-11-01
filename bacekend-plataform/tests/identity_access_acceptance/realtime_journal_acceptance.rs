use redis::aio::ConnectionManager;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn verify(
    redis_manager: &ConnectionManager,
    runtime_pool: &PgPool,
    migration_pool: &PgPool,
    event_payload: &[u8],
    registration_event_id: Uuid,
    legal_identification_number: &str,
) -> ConnectionManager {
    let mut realtime_redis = redis_manager.clone();
    let first_journal_insert =
        platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
            runtime_pool,
            &mut realtime_redis,
            event_payload,
        )
        .await
        .expect("realtime consumer persists the event journal and publishes its Redis signal");
    assert!(first_journal_insert);
    let journal_payload: String =
        sqlx::query_scalar("SELECT payload::text FROM realtime.event_journal WHERE event_id=$1")
            .bind(registration_event_id)
            .fetch_one(migration_pool)
            .await
            .expect("realtime journal contains the registered tenant event");
    assert!(!journal_payload.contains(legal_identification_number));
    let duplicate_journal_insert =
        platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
            runtime_pool,
            &mut realtime_redis,
            event_payload,
        )
        .await
        .expect("redelivered JetStream event is safely deduplicated");
    assert!(!duplicate_journal_insert);
    let journal_event_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM realtime.event_journal WHERE event_id=$1")
            .bind(registration_event_id)
            .fetch_one(migration_pool)
            .await
            .expect("journal stores the event id only once");
    assert_eq!(journal_event_count, 1);

    realtime_redis
}
