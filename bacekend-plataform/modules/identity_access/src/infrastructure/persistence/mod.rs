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

pub async fn claim_outbox_events(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<PendingOutboxEvent>, sqlx::Error> {
    let lease_token = Uuid::new_v4();
    let rows = sqlx::query(
        "WITH candidates AS (\
            SELECT event_id FROM identity_access.outbox_events \
            WHERE published_at IS NULL AND available_at <= now() \
              AND (lease_until IS NULL OR lease_until < now()) \
            ORDER BY occurred_at, event_id FOR UPDATE SKIP LOCKED LIMIT $1\
         ) \
         UPDATE identity_access.outbox_events event \
         SET lease_token=$2, lease_until=now() + interval '30 seconds', attempts=attempts+1 \
         FROM candidates WHERE event.event_id=candidates.event_id \
         RETURNING event.event_id, event.aggregate_id, event.aggregate_version, event.event_type, \
                   event.schema_version, event.audience, event.payload, event.traceparent, event.correlation_id, event.lease_token",
    )
    .bind(limit)
    .bind(lease_token)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(PendingOutboxEvent {
                event_id: row.try_get("event_id")?,
                aggregate_id: row.try_get("aggregate_id")?,
                aggregate_version: row.try_get("aggregate_version")?,
                event_type: row.try_get("event_type")?,
                schema_version: row.try_get("schema_version")?,
                audience: row.try_get("audience")?,
                payload: row.try_get("payload")?,
                traceparent: row.try_get("traceparent")?,
                correlation_id: row.try_get("correlation_id")?,
                lease_token: row.try_get("lease_token")?,
            })
        })
        .collect()
}

pub async fn mark_outbox_published(
    pool: &PgPool,
    event_id: Uuid,
    lease_token: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE identity_access.outbox_events SET published_at=now(), lease_token=NULL, lease_until=NULL, last_error_code=NULL \
         WHERE event_id=$1 AND lease_token=$2 AND published_at IS NULL",
    )
    .bind(event_id)
    .bind(lease_token)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn reschedule_outbox_event(
    pool: &PgPool,
    event_id: Uuid,
    lease_token: Uuid,
    error_code: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE identity_access.outbox_events \
         SET available_at=now() + make_interval(secs => LEAST(300, power(2, LEAST(attempts, 8))::integer)), \
             lease_token=NULL, lease_until=NULL, last_error_code=$3 \
         WHERE event_id=$1 AND lease_token=$2 AND published_at IS NULL",
    )
    .bind(event_id)
    .bind(lease_token)
    .bind(error_code)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn provision_tenant_owner_membership(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    correlation_id: &str,
    traceparent: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true), set_config('app.actor_id',$2,true)")
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *transaction)
        .await?;
    let first_delivery = sqlx::query(
        "INSERT INTO identity_access.consumer_inbox (consumer_name,event_id,correlation_id) \
         VALUES ('tenant_owner_provisioner',$1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(event_id)
    .bind(correlation_id)
    .execute(&mut *transaction)
    .await?
    .rows_affected()
        == 1;
    if !first_delivery {
        transaction.commit().await?;
        return Ok(false);
    }
    let membership_created = sqlx::query(
        "INSERT INTO identity_access.tenant_memberships (tenant_id,user_id,role,status) \
         VALUES ($1,$2,'tenant_owner','active') ON CONFLICT (tenant_id,user_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(user_id)
    .execute(&mut *transaction)
    .await?
    .rows_affected()
        == 1;
    if !membership_created {
        let existing = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM identity_access.tenant_memberships \
             WHERE tenant_id=$1 AND user_id=$2 AND role='tenant_owner' AND status='active')",
        )
        .bind(tenant_id)
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await?;
        if !existing {
            return Err(sqlx::Error::Protocol(
                "tenant owner membership conflicts with an existing membership".to_owned(),
            ));
        }
    }
    if membership_created {
        sqlx::query(
            "INSERT INTO identity_access.audit_events \
             (event_id,actor_kind,actor_id,tenant_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) \
             VALUES ($1,'system',NULL,$2,'tenant.owner_membership.provisioned',$3,'completed',$4,$5,$6)",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(user_id)
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .bind(serde_json::json!({ "source_event_id": event_id }))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.outbox_events \
             (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) \
             VALUES ($1,$2,2,'membership.changed',1,'tenant',$3,$4,$5)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(serde_json::json!({ "tenant_id": tenant_id, "user_id": user_id, "role": "tenant_owner", "status": "active" }))
        .bind(traceparent)
        .bind(correlation_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}

pub const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
    table_name: std::borrow::Cow::Borrowed("identity_access._sqlx_migrations"),
    create_schemas: std::borrow::Cow::Borrowed(&[std::borrow::Cow::Borrowed("identity_access")]),
    ..sqlx::migrate!("infrastructure/persistence/migrations")
};
