use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::infrastructure::persistence::set_tenant_context;

#[derive(Clone)]
pub struct BillingService {
    pub(super) pool: PgPool,
}

pub struct RecordUsage<'a> {
    pub tenant_id: Uuid,
    pub meter_key: &'a str,
    pub source_system: &'a str,
    pub source_event_id: &'a str,
    pub quantity: i64,
    pub unit: &'a str,
    pub occurred_at: DateTime<Utc>,
    pub attributes: Value,
}

pub struct CreateInvoice<'a> {
    pub tenant_id: Uuid,
    pub invoice_number: &'a str,
    pub currency: &'a str,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub due_at: Option<DateTime<Utc>>,
    pub correlation_id: &'a str,
}

pub struct InvoiceLine<'a> {
    pub description: &'a str,
    pub meter_key: Option<&'a str>,
    pub source_usage_event_id: Option<Uuid>,
    pub quantity: i64,
    pub unit: &'a str,
    pub unit_price_minor: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("invalid billing command")]
    InvalidCommand,
    #[error("invoice is not a draft or has no lines")]
    InvoiceNotDraft,
    #[error("idempotency key conflicts with a different usage event")]
    IdempotencyConflict,
    #[error("invoice snapshot could not be reproduced")]
    SnapshotInvalid,
}

impl BillingService {
    pub fn new(pool: PgPool) -> Self { Self { pool } }

    pub async fn record_usage(&self, command: RecordUsage<'_>) -> Result<(Uuid, bool), BillingError> {
        if command.quantity <= 0 || command.attributes.is_null() || !command.attributes.is_object()
            || command.meter_key.is_empty() || command.source_system.is_empty()
            || command.source_event_id.is_empty() || command.unit.is_empty()
        { return Err(BillingError::InvalidCommand); }
        let mut tx = self.pool.begin().await?;
        set_tenant_context(&mut tx, command.tenant_id).await?;
        let id = Uuid::new_v4();
        let inserted = sqlx::query("INSERT INTO platform_billing.usage_events (id,tenant_id,meter_key,source_event_id,source_system,quantity,unit,occurred_at,attributes) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT (tenant_id,source_system,source_event_id) DO NOTHING")
            .bind(id).bind(command.tenant_id).bind(command.meter_key).bind(command.source_event_id)
            .bind(command.source_system).bind(command.quantity).bind(command.unit)
            .bind(command.occurred_at).bind(&command.attributes).execute(&mut *tx).await?;
        if inserted.rows_affected()==0 {
            let row=sqlx::query("SELECT id,meter_key,quantity,unit,occurred_at,attributes FROM platform_billing.usage_events WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=$3")
                .bind(command.tenant_id).bind(command.source_system).bind(command.source_event_id)
                .fetch_one(&mut *tx).await?;
            let same: bool = row.try_get::<String,_>("meter_key")? == command.meter_key
                && row.try_get::<i64,_>("quantity")? == command.quantity
                && row.try_get::<String,_>("unit")? == command.unit
                && row.try_get::<DateTime<Utc>,_>("occurred_at")? == command.occurred_at
                && row.try_get::<Value,_>("attributes")? == command.attributes;
            if !same { return Err(BillingError::IdempotencyConflict); }
            let original: Uuid=row.try_get("id")?;
            tx.commit().await?;
            return Ok((original,true));
        }
        tx.commit().await?;
        Ok((id,false))
    }

    pub async fn create_draft(&self, command: CreateInvoice<'_>) -> Result<Uuid,BillingError> {
        if command.invoice_number.trim().is_empty() || command.currency.len()!=3
            || !command.currency.bytes().all(|b|b.is_ascii_uppercase())
            || command.period_end<=command.period_start || command.correlation_id.is_empty()
        { return Err(BillingError::InvalidCommand); }
        let mut tx=self.pool.begin().await?;
        set_tenant_context(&mut tx,command.tenant_id).await?;
        let id=Uuid::new_v4();
        sqlx::query("INSERT INTO platform_billing.invoices (id,tenant_id,invoice_number,status,currency,period_start,period_end,due_at) VALUES ($1,$2,$3,'draft',$4,$5,$6,$7)")
            .bind(id).bind(command.tenant_id).bind(command.invoice_number).bind(command.currency)
            .bind(command.period_start).bind(command.period_end).bind(command.due_at)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn add_line(&self, tenant_id:Uuid, invoice_id:Uuid, line:InvoiceLine<'_>) -> Result<(),BillingError> {
        if line.description.trim().is_empty() || line.quantity<=0 || line.unit.trim().is_empty()
            || line.unit_price_minor<0
        { return Err(BillingError::InvalidCommand); }
        let mut tx=self.pool.begin().await?;
        set_tenant_context(&mut tx,tenant_id).await?;
        let invoice=sqlx::query("SELECT currency,status FROM platform_billing.invoices WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant_id).bind(invoice_id).fetch_optional(&mut *tx).await?.ok_or(BillingError::InvoiceNotDraft)?;
        let current_status:String=invoice.try_get("status")?;
        if current_status=="issued" {
            let stored:Vec<u8>=sqlx::query_scalar("SELECT snapshot_sha256 FROM platform_billing.invoices WHERE tenant_id=$1 AND id=$2")
                .bind(tenant_id).bind(invoice_id).fetch_one(&mut *tx).await?;
            let hash:[u8;32]=stored.try_into().map_err(|_|BillingError::SnapshotInvalid)?;
            tx.commit().await?;
            return Ok(hash);
        }
        if current_status!="draft" { return Err(BillingError::InvoiceNotDraft); }
        let currency:String=invoice.try_get("currency")?;
        let total=line.quantity.checked_mul(line.unit_price_minor).ok_or(BillingError::InvalidCommand)?;
        let line_number:i32=sqlx::query_scalar("SELECT COALESCE(max(line_number),0)+1 FROM platform_billing.invoice_lines WHERE tenant_id=$1 AND invoice_id=$2")
            .bind(tenant_id).bind(invoice_id).fetch_one(&mut *tx).await?;
        sqlx::query("INSERT INTO platform_billing.invoice_lines (id,tenant_id,invoice_id,line_number,source_usage_event_id,description,meter_key,quantity,unit,unit_price_minor,line_total_minor,currency) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(Uuid::new_v4()).bind(tenant_id).bind(invoice_id).bind(line_number)
            .bind(line.source_usage_event_id).bind(line.description).bind(line.meter_key)
            .bind(line.quantity).bind(line.unit).bind(line.unit_price_minor).bind(total).bind(currency)
            .execute(&mut *tx).await?;
        sqlx::query("UPDATE platform_billing.invoices SET subtotal_minor=subtotal_minor+$3,total_minor=total_minor+$3,updated_at=now() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant_id).bind(invoice_id).bind(total).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn issue_invoice(&self, tenant_id:Uuid, invoice_id:Uuid, correlation_id:&str) -> Result<[u8;32],BillingError> {
        if correlation_id.is_empty() { return Err(BillingError::InvalidCommand); }
        let mut tx=self.pool.begin().await?;
        set_tenant_context(&mut tx,tenant_id).await?;
        let invoice=sqlx::query("SELECT invoice_number,currency,period_start,period_end,due_at,total_minor,subtotal_minor,status FROM platform_billing.invoices WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant_id).bind(invoice_id).fetch_optional(&mut *tx).await?.ok_or(BillingError::InvoiceNotDraft)?;
        if invoice.try_get::<String,_>("status")?!="draft" { return Err(BillingError::InvoiceNotDraft); }
        let lines=sqlx::query("SELECT line_number,description,meter_key,quantity,unit,unit_price_minor,line_total_minor,currency,source_usage_event_id FROM platform_billing.invoice_lines WHERE tenant_id=$1 AND invoice_id=$2 ORDER BY line_number")
            .bind(tenant_id).bind(invoice_id).fetch_all(&mut *tx).await?;
        if lines.is_empty() { return Err(BillingError::InvoiceNotDraft); }
        let line_values:Vec<Value>=lines.iter().map(|row| Ok(json!({
            "line_number":row.try_get::<i32,_>("line_number")?,
            "description":row.try_get::<String,_>("description")?,
            "meter_key":row.try_get::<Option<String>,_>("meter_key")?,
            "quantity":row.try_get::<i64,_>("quantity")?,
            "unit":row.try_get::<String,_>("unit")?,
            "unit_price_minor":row.try_get::<i64,_>("unit_price_minor")?,
            "line_total_minor":row.try_get::<i64,_>("line_total_minor")?,
            "currency":row.try_get::<String,_>("currency")?,
            "source_usage_event_id":row.try_get::<Option<Uuid>,_>("source_usage_event_id")?
        }))).collect::<Result<_,sqlx::Error>>()?;
        let snapshot=json!({
            "snapshot_version":1,"invoice_id":invoice_id,"tenant_id":tenant_id,
            "invoice_number":invoice.try_get::<String,_>("invoice_number")?,
            "currency":invoice.try_get::<String,_>("currency")?,
            "period_start":invoice.try_get::<DateTime<Utc>,_>("period_start")?,
            "period_end":invoice.try_get::<DateTime<Utc>,_>("period_end")?,
            "due_at":invoice.try_get::<Option<DateTime<Utc>>,_>("due_at")?,
            "subtotal_minor":invoice.try_get::<i64,_>("subtotal_minor")?,
            "total_minor":invoice.try_get::<i64,_>("total_minor")?,
            "lines":line_values
        });
        let canonical=serde_json::to_vec(&snapshot).map_err(|_|BillingError::SnapshotInvalid)?;
        let hash:[u8;32]=Sha256::digest(canonical).into();
        let total:i64=invoice.try_get("total_minor")?;
        sqlx::query("UPDATE platform_billing.invoices SET status='issued',issued_at=now(),snapshot_version=1,snapshot=$3,snapshot_sha256=$4,aggregate_version=aggregate_version+1,updated_at=now() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant_id).bind(invoice_id).bind(&snapshot).bind(hash.as_slice()).execute(&mut *tx).await?;
        if total>0 {
            let journal_id=Uuid::new_v4();
            sqlx::query("INSERT INTO platform_billing.ledger_journals (id,tenant_id,invoice_id,journal_key,currency) VALUES ($1,$2,$3,$4,$5)")
                .bind(journal_id).bind(tenant_id).bind(invoice_id).bind(format!("invoice-issued:{invoice_id}"))
                .bind(invoice.try_get::<String,_>("currency")?).execute(&mut *tx).await?;
            for (entry,account,direction) in [(1_i16,"accounts_receivable","debit"),(2_i16,"saas_revenue","credit")] {
                sqlx::query("INSERT INTO platform_billing.ledger_entries (id,tenant_id,journal_id,entry_number,account_code,direction,amount_minor,currency) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
                    .bind(Uuid::new_v4()).bind(tenant_id).bind(journal_id).bind(entry).bind(account).bind(direction).bind(total)
                    .bind(invoice.try_get::<String,_>("currency")?).execute(&mut *tx).await?;
            }
        }
        sqlx::query("INSERT INTO platform_billing.outbox_events (event_id,tenant_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,$3,2,'billing.invoice.issued',1,'fiscal_integration',$4,$5)")
            .bind(Uuid::new_v4()).bind(tenant_id).bind(invoice_id)
            .bind(json!({"invoice_id":invoice_id,"tenant_id":tenant_id,"snapshot_version":1,"snapshot":snapshot,"snapshot_sha256":hex(&hash)}))
            .bind(correlation_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(hash)
    }
}

fn hex(bytes:&[u8])->String {
    const HEX:&[u8;16]=b"0123456789abcdef";
    let mut out=String::with_capacity(bytes.len()*2);
    for byte in bytes { out.push(HEX[(byte>>4) as usize] as char); out.push(HEX[(byte&15) as usize] as char); }
    out
}
