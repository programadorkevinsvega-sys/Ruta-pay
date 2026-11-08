use sha2::{Digest,Sha256};
use sqlx::{PgPool,Row};
use uuid::Uuid;

use super::persistence::set_tenant_context;

pub struct CreateCollectionAttempt<'a>{
    pub tenant_id:Uuid,pub invoice_id:Uuid,pub provider_code:&'a str,
    pub idempotency_key:&'a str,pub amount_minor:i64,pub currency:&'a str,
}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct CollectionAttempt { pub id:Uuid,pub attempt_number:i32,pub status:String,pub replayed:bool }

pub async fn create_attempt(pool:&PgPool,command:CreateCollectionAttempt<'_>)->Result<CollectionAttempt,sqlx::Error>{
    if command.amount_minor<=0 || command.currency.len()!=3
        || !command.currency.bytes().all(|b|b.is_ascii_uppercase())
        || command.idempotency_key.is_empty() || command.provider_code.is_empty()
    { return Err(sqlx::Error::Protocol("invalid platform collection attempt".into())); }
    let mut tx=pool.begin().await?;
    set_tenant_context(&mut tx,command.tenant_id).await?;
    let invoice=sqlx::query("SELECT currency,total_minor,status FROM platform_billing.invoices WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(command.tenant_id).bind(command.invoice_id).fetch_one(&mut *tx).await?;
    if invoice.try_get::<String,_>("status")?!="issued"
        || invoice.try_get::<i64,_>("total_minor")?!=command.amount_minor
        || invoice.try_get::<String,_>("currency")?!=command.currency {
        return Err(sqlx::Error::Protocol("collection amount or currency does not match issued invoice".into()));
    }
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
    let id=Uuid::new_v4();
    sqlx::query("INSERT INTO platform_billing.collection_attempts (id,tenant_id,invoice_id,attempt_number,provider_code,idempotency_key,request_sha256,status,amount_minor,currency) VALUES ($1,$2,$3,1,$4,$5,$6,'created',$7,$8)")
        .bind(id).bind(command.tenant_id).bind(command.invoice_id).bind(command.provider_code)
        .bind(command.idempotency_key).bind(hash.as_slice()).bind(command.amount_minor).bind(command.currency)
        .execute(&mut *tx).await?;
    sqlx::query("INSERT INTO platform_billing.outbox_events (event_id,tenant_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,$3,1,'billing.collection.created',1,'platform',$4,$5)")
        .bind(Uuid::new_v4()).bind(command.tenant_id).bind(id)
        .bind(serde_json::json!({"collection_attempt_id":id,"invoice_id":command.invoice_id,"amount_minor":command.amount_minor,"currency":command.currency,"provider_code":command.provider_code}))
        .bind(format!("collection:{id}")).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(CollectionAttempt{id,attempt_number:1,status:"created".into(),replayed:false})
}

pub async fn transition_attempt(
    pool:&PgPool,tenant:Uuid,attempt_id:Uuid,next_status:&str,provider_reference:Option<&str>,
)->Result<bool,sqlx::Error>{
    if !matches!(next_status,"submitted"|"processing"|"succeeded"|"failed"|"outcome_unknown") {
        return Err(sqlx::Error::Protocol("invalid platform collection status".into()));
    }
    if provider_reference.is_some_and(|value|value.is_empty()||value.len()>160||value.bytes().any(|b|b.is_ascii_control())) {
        return Err(sqlx::Error::Protocol("invalid provider reference".into()));
    }
    let mut tx=pool.begin().await?;
    set_tenant_context(&mut tx,tenant).await?;
    let row=sqlx::query("SELECT id,invoice_id,status,aggregate_version,amount_minor,currency FROM platform_billing.collection_attempts WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant).bind(attempt_id).fetch_one(&mut *tx).await?;
    let current:String=row.try_get("status")?;
    if current==next_status { tx.commit().await?; return Ok(false); }
    let version:i64=row.try_get("aggregate_version")?+1;
    let changed=sqlx::query("UPDATE platform_billing.collection_attempts SET status=$3,provider_reference=COALESCE(provider_reference,$4) WHERE tenant_id=$1 AND id=$2")
        .bind(tenant).bind(attempt_id).bind(next_status).bind(provider_reference)
        .execute(&mut *tx).await?;
    if changed.rows_affected()!=1 { return Err(sqlx::Error::RowNotFound); }
    let invoice_id:Uuid=row.try_get("invoice_id")?;
    sqlx::query("INSERT INTO platform_billing.outbox_events (event_id,tenant_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,$3,$4,'billing.collection.status_changed',1,'platform',$5,$6)")
        .bind(Uuid::new_v4()).bind(tenant).bind(attempt_id).bind(version)
        .bind(serde_json::json!({"collection_attempt_id":attempt_id,"invoice_id":invoice_id,"previous_status":current,"status":next_status,"provider_reference":provider_reference}))
        .bind(format!("collection:{attempt_id}")).execute(&mut *tx).await?;
    if next_status=="succeeded" {
        let amount:i64=row.try_get("amount_minor")?;
        let currency:String=row.try_get("currency")?;
        let journal=Uuid::new_v4();
        let inserted=sqlx::query("INSERT INTO platform_billing.ledger_journals (id,tenant_id,invoice_id,journal_key,currency) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (tenant_id,journal_key) DO NOTHING")
            .bind(journal).bind(tenant).bind(invoice_id).bind(format!("collection-succeeded:{attempt_id}")).bind(&currency)
            .execute(&mut *tx).await?;
        if inserted.rows_affected()==1 {
            for (entry,account,direction) in [(1_i16,"cash_clearing","debit"),(2_i16,"accounts_receivable","credit")] {
                sqlx::query("INSERT INTO platform_billing.ledger_entries (id,tenant_id,journal_id,entry_number,account_code,direction,amount_minor,currency) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
                    .bind(Uuid::new_v4()).bind(tenant).bind(journal).bind(entry).bind(account).bind(direction).bind(amount).bind(&currency)
                    .execute(&mut *tx).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(true)
}
