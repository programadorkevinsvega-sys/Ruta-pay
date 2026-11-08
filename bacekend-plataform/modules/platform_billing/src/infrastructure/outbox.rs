use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct PendingBillingEvent {
    pub event_id: Uuid,
    pub tenant_id: Uuid,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub event_type: String,
    pub schema_version: i16,
    pub audience: String,
    pub payload: serde_json::Value,
    pub correlation_id: String,
    pub lease_token: Uuid,
}

pub async fn claim(pool:&PgPool,limit:i64)->Result<Vec<PendingBillingEvent>,sqlx::Error>{
    if !(1..=256).contains(&limit) { return Ok(Vec::new()); }
    let mut tx=pool.begin().await?;
    sqlx::query("SELECT set_config('app.operator_scope','true',true)").execute(&mut *tx).await?;
    let token=Uuid::new_v4();
    let rows=sqlx::query("WITH candidates AS (SELECT event_id FROM platform_billing.outbox_events WHERE published_at IS NULL AND failed_at IS NULL AND available_at<=now() AND (lease_until IS NULL OR lease_until<now()) ORDER BY occurred_at,event_id FOR UPDATE SKIP LOCKED LIMIT $1) UPDATE platform_billing.outbox_events e SET lease_token=$2,lease_until=now()+interval '30 seconds',attempts=attempts+1 FROM candidates c WHERE e.event_id=c.event_id RETURNING e.event_id,e.tenant_id,e.aggregate_id,e.aggregate_version,e.event_type,e.schema_version,e.audience,e.payload,e.correlation_id,e.lease_token")
        .bind(limit).bind(token).fetch_all(&mut *tx).await?;
    let events=rows.iter().map(|r|Ok(PendingBillingEvent{
        event_id:r.try_get("event_id")?,tenant_id:r.try_get("tenant_id")?,
        aggregate_id:r.try_get("aggregate_id")?,aggregate_version:r.try_get("aggregate_version")?,
        event_type:r.try_get("event_type")?,schema_version:r.try_get("schema_version")?,
        audience:r.try_get("audience")?,payload:r.try_get("payload")?,
        correlation_id:r.try_get("correlation_id")?,lease_token:r.try_get("lease_token")?
    })).collect::<Result<Vec<_>,sqlx::Error>>()?;
    tx.commit().await?;
    Ok(events)
}

pub async fn acknowledge(pool:&PgPool,event_id:Uuid,token:Uuid)->Result<bool,sqlx::Error>{
    let mut tx=pool.begin().await?;
    sqlx::query("SELECT set_config('app.operator_scope','true',true)").execute(&mut *tx).await?;
    let result=sqlx::query("UPDATE platform_billing.outbox_events SET published_at=now(),lease_token=NULL,lease_until=NULL,last_error_code=NULL WHERE event_id=$1 AND lease_token=$2 AND lease_until>now() AND published_at IS NULL AND failed_at IS NULL")
        .bind(event_id).bind(token).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected()==1)
}

pub async fn retry(pool:&PgPool,event_id:Uuid,token:Uuid,error:&str)->Result<bool,sqlx::Error>{
    if error.is_empty() || error.len()>64 || !error.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'_') { return Ok(false); }
    let mut tx=pool.begin().await?;
    sqlx::query("SELECT set_config('app.operator_scope','true',true)").execute(&mut *tx).await?;
    let result=sqlx::query("UPDATE platform_billing.outbox_events SET available_at=now()+make_interval(secs=>LEAST(900,GREATEST(1,ceil(power(2.0::double precision,LEAST(attempts,9)::double precision)*(0.5+random()))::integer))),lease_token=NULL,lease_until=NULL,last_error_code=$3 WHERE event_id=$1 AND lease_token=$2 AND lease_until>now() AND published_at IS NULL AND failed_at IS NULL")
        .bind(event_id).bind(token).bind(error).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected()==1)
}
