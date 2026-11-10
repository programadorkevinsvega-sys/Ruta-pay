use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::{BillingError, BillingService, RecordUsage};
use crate::infrastructure::persistence::set_tenant_context;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageInboxResult {
    Processed { duplicate_usage: bool },
    DuplicateProcessed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSummary {
    pub tenant_id: Uuid,
    pub meter_key: String,
    pub unit: String,
    pub quantity: i64,
    pub event_count: i64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

pub(super) async fn insert_usage(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    command: &RecordUsage<'_>,
    occurred_at: DateTime<Utc>,
) -> Result<(Uuid, bool), BillingError> {
    let id = Uuid::new_v4();
    let inserted = sqlx::query("INSERT INTO platform_billing.usage_events (id,tenant_id,meter_key,source_event_id,source_system,quantity,unit,occurred_at,attributes) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT (tenant_id,source_system,source_event_id) DO NOTHING")
        .bind(id)
        .bind(command.tenant_id)
        .bind(command.meter_key)
        .bind(command.source_event_id)
        .bind(command.source_system)
        .bind(command.quantity)
        .bind(command.unit)
        .bind(occurred_at)
        .bind(&command.attributes)
        .execute(&mut **tx)
        .await?;

    if inserted.rows_affected() == 1 {
        return Ok((id, false));
    }
    let row = sqlx::query("SELECT id,meter_key,quantity,unit,occurred_at,attributes FROM platform_billing.usage_events WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=$3")
        .bind(command.tenant_id)
        .bind(command.source_system)
        .bind(command.source_event_id)
        .fetch_one(&mut **tx)
        .await?;
    let same = row.try_get::<String, _>("meter_key")? == command.meter_key
        && row.try_get::<i64, _>("quantity")? == command.quantity
        && row.try_get::<String, _>("unit")? == command.unit
        && row.try_get::<DateTime<Utc>, _>("occurred_at")? == occurred_at
        && row.try_get::<Value, _>("attributes")? == command.attributes;
    if !same {
        return Err(BillingError::IdempotencyConflict);
    }
    Ok((row.try_get::<Uuid, _>("id")?, true))
}

impl BillingService {
    /// Atomically records an event receipt, usage row, and processed disposition.
    /// The common successful path uses one PostgreSQL transaction.
    pub async fn record_usage_from_event(
        &self,
        command: RecordUsage<'_>,
        producer: &str,
        event_id: Uuid,
        event_type: &str,
        schema_version: i16,
        payload: &Value,
    ) -> Result<UsageInboxResult, BillingError> {
        if command.quantity <= 0
            || command.attributes.is_null()
            || !command.attributes.is_object()
            || command.meter_key.is_empty()
            || command.source_system.is_empty()
            || command.source_event_id.is_empty()
            || command.unit.is_empty()
            || producer.is_empty()
            || event_type.is_empty()
            || schema_version <= 0
            || !payload.is_object()
        {
            return Err(BillingError::InvalidCommand);
        }
        let occurred_at =
            DateTime::<Utc>::from_timestamp_micros(command.occurred_at.timestamp_micros())
                .ok_or(BillingError::InvalidCommand)?;
        let payload_bytes =
            serde_json::to_vec(payload).map_err(|_| BillingError::InvalidCommand)?;
        let payload_digest: [u8; 32] = Sha256::digest(payload_bytes).into();
        let mut tx = self.pool.begin().await?;
        set_tenant_context(&mut tx, command.tenant_id).await?;

        let inbox_id = Uuid::new_v4();
        let inserted = sqlx::query("INSERT INTO platform_billing.inbox_events (id,tenant_id,producer_id,event_id,event_type,schema_version,payload_sha256,disposition) VALUES ($1,$2,$3,$4,$5,$6,$7,'received') ON CONFLICT (tenant_id,producer_id,event_id) DO NOTHING")
            .bind(inbox_id)
            .bind(command.tenant_id)
            .bind(producer)
            .bind(event_id)
            .bind(event_type)
            .bind(schema_version)
            .bind(payload_digest.as_slice())
            .execute(&mut *tx)
            .await?;

        let inbox_id = if inserted.rows_affected() == 1 {
            inbox_id
        } else {
            let row = sqlx::query("SELECT id,event_type,schema_version,payload_sha256,disposition FROM platform_billing.inbox_events WHERE tenant_id=$1 AND producer_id=$2 AND event_id=$3")
                .bind(command.tenant_id)
                .bind(producer)
                .bind(event_id)
                .fetch_one(&mut *tx)
                .await?;
            let same = row.try_get::<String, _>("event_type")? == event_type
                && row.try_get::<i16, _>("schema_version")? == schema_version
                && row.try_get::<Vec<u8>, _>("payload_sha256")? == payload_digest;
            if !same {
                return Err(BillingError::InboxIdempotencyConflict);
            }
            if row.try_get::<String, _>("disposition")? == "processed" {
                tx.commit().await?;
                return Ok(UsageInboxResult::DuplicateProcessed);
            }
            row.try_get::<Uuid, _>("id")?
        };

        let (_usage_id, duplicate_usage) = insert_usage(&mut tx, &command, occurred_at).await?;
        let marked = sqlx::query("UPDATE platform_billing.inbox_events SET disposition='processed',processed_at=COALESCE(processed_at,now()) WHERE tenant_id=$1 AND id=$2 AND disposition='received'")
            .bind(command.tenant_id)
            .bind(inbox_id)
            .execute(&mut *tx)
            .await?;
        if marked.rows_affected() != 1 {
            return Err(BillingError::InvalidCommand);
        }
        tx.commit().await?;
        Ok(UsageInboxResult::Processed { duplicate_usage })
    }

    /// Records usage and completes its inbox receipt in one PostgreSQL transaction.
    pub async fn record_usage_with_inbox(
        &self,
        command: RecordUsage<'_>,
        inbox_id: Uuid,
    ) -> Result<(Uuid, bool), BillingError> {
        if command.quantity <= 0
            || command.attributes.is_null()
            || !command.attributes.is_object()
            || command.meter_key.is_empty()
            || command.source_system.is_empty()
            || command.source_event_id.is_empty()
            || command.unit.is_empty()
        {
            return Err(BillingError::InvalidCommand);
        }

        let occurred_at =
            DateTime::<Utc>::from_timestamp_micros(command.occurred_at.timestamp_micros())
                .ok_or(BillingError::InvalidCommand)?;
        let mut tx = self.pool.begin().await?;
        set_tenant_context(&mut tx, command.tenant_id).await?;
        let (usage_id, duplicate) = insert_usage(&mut tx, &command, occurred_at).await?;

        let marked = sqlx::query("UPDATE platform_billing.inbox_events SET disposition='processed',processed_at=COALESCE(processed_at,now()) WHERE tenant_id=$1 AND id=$2 AND disposition='received'")
            .bind(command.tenant_id)
            .bind(inbox_id)
            .execute(&mut *tx)
            .await?;
        if marked.rows_affected() == 0 {
            let disposition: Option<String> = sqlx::query_scalar(
                "SELECT disposition FROM platform_billing.inbox_events WHERE tenant_id=$1 AND id=$2",
            )
            .bind(command.tenant_id)
            .bind(inbox_id)
            .fetch_optional(&mut *tx)
            .await?;
            if disposition.as_deref() != Some("processed") {
                return Err(BillingError::InvalidCommand);
            }
        }

        tx.commit().await?;
        Ok((usage_id, duplicate))
    }

    pub async fn summarize_usage(
        &self,
        tenant_id: Uuid,
        meter_key: &str,
        unit: &str,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<UsageSummary, BillingError> {
        if meter_key.is_empty() || unit.is_empty() || period_end <= period_start {
            return Err(BillingError::InvalidCommand);
        }
        let mut tx = self.pool.begin().await?;
        set_tenant_context(&mut tx, tenant_id).await?;
        let row=sqlx::query("SELECT COALESCE(sum(quantity),0)::bigint AS quantity,count(*)::bigint AS event_count FROM platform_billing.usage_events WHERE tenant_id=$1 AND meter_key=$2 AND unit=$3 AND occurred_at >= $4 AND occurred_at < $5")
            .bind(tenant_id).bind(meter_key).bind(unit).bind(period_start).bind(period_end)
            .fetch_one(&mut *tx).await?;
        let result = UsageSummary {
            tenant_id,
            meter_key: meter_key.to_owned(),
            unit: unit.to_owned(),
            quantity: row.try_get("quantity")?,
            event_count: row.try_get("event_count")?,
            period_start,
            period_end,
        };
        tx.commit().await?;
        Ok(result)
    }
}
