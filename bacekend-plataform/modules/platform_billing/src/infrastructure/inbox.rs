use sha2::{Digest,Sha256};
use sqlx::{PgPool,Row};
use uuid::Uuid;

use super::persistence::set_tenant_context;

#[derive(Debug,PartialEq,Eq)]
pub enum InboxDisposition { Inserted(Uuid), Duplicate(Uuid) }

pub async fn receive(
    pool:&PgPool,tenant:Uuid,producer:&str,event_id:Uuid,event_type:&str,
    schema_version:i16,payload:&serde_json::Value,
)->Result<InboxDisposition,sqlx::Error>{
    let bytes=serde_json::to_vec(payload).map_err(|e|sqlx::Error::Protocol(e.to_string()))?;
    let digest:[u8;32]=Sha256::digest(bytes).into();
    let mut tx=pool.begin().await?;
    set_tenant_context(&mut tx,tenant).await?;
    let id=Uuid::new_v4();
    let inserted=sqlx::query("INSERT INTO platform_billing.inbox_events (id,tenant_id,producer_id,event_id,event_type,schema_version,payload_sha256,disposition) VALUES ($1,$2,$3,$4,$5,$6,$7,'received') ON CONFLICT (producer_id,event_id) DO NOTHING")
        .bind(id).bind(tenant).bind(producer).bind(event_id).bind(event_type).bind(schema_version).bind(digest.as_slice()).execute(&mut *tx).await?;
    if inserted.rows_affected()==1 { tx.commit().await?; return Ok(InboxDisposition::Inserted(id)); }
    let row=sqlx::query("SELECT id,tenant_id,event_type,schema_version,payload_sha256 FROM platform_billing.inbox_events WHERE producer_id=$1 AND event_id=$2")
        .bind(producer).bind(event_id).fetch_one(&mut *tx).await?;
    let same:bool=row.try_get::<Uuid,_>("tenant_id")?==tenant
        && row.try_get::<String,_>("event_type")?==event_type
        && row.try_get::<i16,_>("schema_version")?==schema_version
        && row.try_get::<Vec<u8>,_>("payload_sha256")?==digest;
    if !same { return Err(sqlx::Error::Protocol("billing inbox idempotency conflict".into())); }
    let original=row.try_get("id")?;
    tx.commit().await?;
    Ok(InboxDisposition::Duplicate(original))
}

pub async fn mark_processed(pool:&PgPool,tenant:Uuid,id:Uuid)->Result<bool,sqlx::Error>{
    let mut tx=pool.begin().await?;
    set_tenant_context(&mut tx,tenant).await?;
    let result=sqlx::query("UPDATE platform_billing.inbox_events SET disposition='processed',processed_at=now() WHERE tenant_id=$1 AND id=$2 AND disposition='received'")
        .bind(tenant).bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected()==1)
}
