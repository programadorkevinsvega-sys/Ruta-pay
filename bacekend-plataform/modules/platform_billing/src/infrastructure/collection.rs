use sha2::{Digest,Sha256};
use sqlx::{PgPool,Row};
use uuid::Uuid;

use super::persistence::set_tenant_context;

pub struct CreateCollectionAttempt<'a>{
    pub tenant_id:Uuid,pub invoice_id:Uuid,pub provider_code:&'a str,
    pub idempotency_key:&'a str,pub amount_minor:i64,pub currency:&'a str,
}
pub struct CollectionAttempt { pub id:Uuid,pub attempt_number:i32,pub status:String,pub replayed:bool }

pub async fn create_attempt(pool:&PgPool,command:CreateCollectionAttempt<'_>)->Result<CollectionAttempt,sqlx::Error>{
    if command.amount_minor<=0 || command.currency.len()!=3
        || !command.currency.bytes().all(|b|b.is_ascii_uppercase())
        || command.idempotency_key.is_empty() || command.provider_code.is_empty()
    { return Err(sqlx::Error::Protocol("invalid platform collection attempt".into())); }
    let mut tx=pool.begin().await?;
    set_tenant_context(&mut tx,command.tenant_id).await?;
    sqlx::query("SELECT id FROM platform_billing.invoices WHERE tenant_id=$1 AND id=$2 AND status='issued' FOR UPDATE")
        .bind(command.tenant_id).bind(command.invoice_id).fetch_one(&mut *tx).await?;
    let request=serde_json::json!({"provider":command.provider_code,"amount_minor":command.amount_minor,"currency":command.currency});
    let hash:[u8;32]=Sha256::digest(serde_json::to_vec(&request).map_err(|e|sqlx::Error::Protocol(e.to_string()))?).into();
    let existing=sqlx::query("SELECT id,attempt_number,status,request_sha256,amount_minor,currency,provider_code FROM platform_billing.collection_attempts WHERE tenant_id=$1 AND invoice_id=$2 AND idempotency_key=$3")
        .bind(command.tenant_id).bind(command.invoice_id).bind(command.idempotency_key).fetch_optional(&mut *tx).await?;
    if let Some(row)=existing {
        let same=row.try_get::<Vec<u8>,_>("request_sha256")?==hash
            && row.try_get::<i64,_>("amount_minor")?==command.amount_minor
            && row.try_get::<String,_>("currency")?==command.currency
            && row.try_get::<String,_>("provider_code")?==command.provider_code;
        if !same { return Err(sqlx::Error::Protocol("platform collection idempotency conflict".into())); }
        let result=CollectionAttempt{id:row.try_get("id")?,attempt_number:row.try_get("attempt_number")?,status:row.try_get("status")?,replayed:true};
        tx.commit().await?;
        return Ok(result);
    }
    let number:i32=sqlx::query_scalar("SELECT COALESCE(max(attempt_number),0)+1 FROM platform_billing.collection_attempts WHERE tenant_id=$1 AND invoice_id=$2")
        .bind(command.tenant_id).bind(command.invoice_id).fetch_one(&mut *tx).await?;
    let id=Uuid::new_v4();
    sqlx::query("INSERT INTO platform_billing.collection_attempts (id,tenant_id,invoice_id,attempt_number,provider_code,idempotency_key,request_sha256,status,amount_minor,currency) VALUES ($1,$2,$3,$4,$5,$6,$7,'created',$8,$9)")
        .bind(id).bind(command.tenant_id).bind(command.invoice_id).bind(number).bind(command.provider_code)
        .bind(command.idempotency_key).bind(hash.as_slice()).bind(command.amount_minor).bind(command.currency)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(CollectionAttempt{id,attempt_number:number,status:"created".into(),replayed:false})
}
