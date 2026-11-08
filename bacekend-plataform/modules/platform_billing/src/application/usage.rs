use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::{BillingError, BillingService, RecordUsage};
use crate::infrastructure::persistence::set_tenant_context;

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

impl BillingService {
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

        let mut tx = self.pool.begin().await?;
        set_tenant_context(&mut tx, command.tenant_id).await?;
        let id = Uuid::new_v4();
        let inserted = sqlx::query("INSERT INTO platform_billing.usage_events (id,tenant_id,meter_key,source_event_id,source_system,quantity,unit,occurred_at,attributes) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT (tenant_id,source_system,source_event_id) DO NOTHING")
            .bind(id)
            .bind(command.tenant_id)
            .bind(command.meter_key)
            .bind(command.source_event_id)
            .bind(command.source_system)
            .bind(command.quantity)
            .bind(command.unit)
            .bind(command.occurred_at)
            .bind(&command.attributes)
            .execute(&mut *tx)
            .await?;

        let (usage_id, duplicate) = if inserted.rows_affected() == 1 {
            (id, false)
        } else {
            let row = sqlx::query("SELECT id,meter_key,quantity,unit,occurred_at,attributes FROM platform_billing.usage_events WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=$3")
                .bind(command.tenant_id)
                .bind(command.source_system)
                .bind(command.source_event_id)
                .fetch_one(&mut *tx)
                .await?;
            let same = row.try_get::<String, _>("meter_key")? == command.meter_key
                && row.try_get::<i64, _>("quantity")? == command.quantity
                && row.try_get::<String, _>("unit")? == command.unit
                && row.try_get::<DateTime<Utc>, _>("occurred_at")? == command.occurred_at
                && row.try_get::<Value, _>("attributes")? == command.attributes;
            if !same {
                return Err(BillingError::IdempotencyConflict);
            }
            (row.try_get::<Uuid, _>("id")?, true)
        };

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
