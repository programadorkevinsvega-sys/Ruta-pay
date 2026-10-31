use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct PendingOutboxEvent {
    pub event_id: Uuid,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub event_type: String,
    pub schema_version: i16,
    pub audience: String,
    pub payload: serde_json::Value,
    pub traceparent: Option<String>,
    pub correlation_id: String,
    pub lease_token: Uuid,
}

pub async fn claim_outbox_events(pool: &PgPool, limit: i64) -> Result<Vec<PendingOutboxEvent>, sqlx::Error> {
    let lease_token = Uuid::new_v4();
    let rows = sqlx::query(
        "WITH candidates AS (SELECT event_id FROM catalog_pricing.outbox_events WHERE published_at IS NULL AND available_at <= now() AND (lease_until IS NULL OR lease_until < now()) ORDER BY occurred_at,event_id FOR UPDATE SKIP LOCKED LIMIT $1) UPDATE catalog_pricing.outbox_events event SET lease_token=$2,lease_until=now()+interval '30 seconds',attempts=attempts+1 FROM candidates WHERE event.event_id=candidates.event_id RETURNING event.event_id,event.aggregate_id,event.aggregate_version,event.event_type,event.schema_version,event.audience,event.payload,event.traceparent,event.correlation_id,event.lease_token"
    ).bind(limit).bind(lease_token).fetch_all(pool).await?;
    rows.iter().map(|row| Ok(PendingOutboxEvent {
        event_id:row.try_get("event_id")?, aggregate_id:row.try_get("aggregate_id")?,
        aggregate_version:row.try_get("aggregate_version")?, event_type:row.try_get("event_type")?,
        schema_version:row.try_get("schema_version")?, audience:row.try_get("audience")?,
        payload:row.try_get("payload")?, traceparent:row.try_get("traceparent")?,
        correlation_id:row.try_get("correlation_id")?, lease_token:row.try_get("lease_token")?,
    })).collect()
}

pub async fn mark_outbox_published(pool: &PgPool, event_id: Uuid, lease_token: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE catalog_pricing.outbox_events SET published_at=now(),lease_token=NULL,lease_until=NULL,last_error_code=NULL WHERE event_id=$1 AND lease_token=$2 AND published_at IS NULL")
        .bind(event_id).bind(lease_token).execute(pool).await?;
    Ok(result.rows_affected()==1)
}

pub async fn reschedule_outbox_event(pool: &PgPool, event_id: Uuid, lease_token: Uuid, error_code: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE catalog_pricing.outbox_events SET available_at=now()+make_interval(secs=>LEAST(300,power(2,LEAST(attempts,8))::integer)),lease_token=NULL,lease_until=NULL,last_error_code=$3 WHERE event_id=$1 AND lease_token=$2 AND published_at IS NULL")
        .bind(event_id).bind(lease_token).bind(error_code).execute(pool).await?;
    Ok(result.rows_affected()==1)
}

pub const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
    table_name: std::borrow::Cow::Borrowed("catalog_pricing._sqlx_migrations"),
    create_schemas: std::borrow::Cow::Borrowed(&[std::borrow::Cow::Borrowed("catalog_pricing")]),
    ..sqlx::migrate!("infrastructure/persistence/migrations")
};
