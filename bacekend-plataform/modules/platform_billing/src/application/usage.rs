use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::{BillingError, BillingService};
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
