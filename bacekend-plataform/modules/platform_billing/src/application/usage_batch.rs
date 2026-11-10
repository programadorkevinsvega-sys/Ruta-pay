use chrono::{DateTime, Utc};
use metrics::histogram;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Connection, Postgres, QueryBuilder, Row};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use super::{BillingError, BillingService};
use crate::infrastructure::persistence::set_tenant_context;

#[derive(Debug, Clone)]
pub struct UsageBatchEvent {
    pub tenant_id: Uuid,
    pub producer: String,
    pub event_id: Uuid,
    pub event_type: String,
    pub schema_version: i16,
    pub payload: Value,
    pub source_system: String,
    pub source_event_id: String,
    pub meter_key: String,
    pub quantity: i64,
    pub unit: String,
    pub occurred_at: DateTime<Utc>,
    pub attributes: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageBatchDisposition {
    Processed,
    DuplicateProcessed,
}

impl BillingService {
    /// Persists one tenant group atomically. Callers must never mix tenant IDs.
    /// Inbox and usage rows are inserted in set-based statements and ACK is left
    /// to the caller after this method returns (which is after COMMIT).
    pub async fn record_usage_batch_for_tenant(
        &self,
        tenant_id: Uuid,
        events: &[UsageBatchEvent],
    ) -> Result<Vec<UsageBatchDisposition>, BillingError> {
        if events.is_empty()
            || events.len() > 512
            || events.iter().any(|event| {
                event.tenant_id != tenant_id
                    || event.producer.is_empty()
                    || event.event_type.is_empty()
                    || event.schema_version <= 0
                    || event.source_system.is_empty()
                    || event.source_event_id.is_empty()
                    || event.meter_key.is_empty()
                    || event.meter_key.len() > 96
                    || event.unit.is_empty()
                    || event.unit.len() > 32
                    || event.quantity <= 0
                    || !event.payload.is_object()
                    || !event.attributes.is_object()
                    || event.producer != events[0].producer
                    || event.source_system != events[0].source_system
            })
        {
            return Err(BillingError::InvalidCommand);
        }

        let digests = events
            .iter()
            .map(|event| {
                serde_json::to_vec(&event.payload)
                    .map(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)))
                    .map_err(|_| BillingError::InvalidCommand)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let acquire_started = Instant::now();
        let mut connection = self.pool.acquire().await?;
        histogram!("platform_billing_usage_pool_acquire_seconds")
            .record(acquire_started.elapsed().as_secs_f64());
        let mut tx = connection.begin().await?;
        let sql_started = Instant::now();
        set_tenant_context(&mut tx, tenant_id).await?;

        let mut inbox_insert = QueryBuilder::<Postgres>::new(
            "INSERT INTO platform_billing.inbox_events              (id,tenant_id,producer_id,event_id,event_type,schema_version,payload_sha256,disposition) ",
        );
        inbox_insert.push_values(
            events.iter().zip(digests.iter()),
            |mut row, (event, digest)| {
                row.push_bind(Uuid::new_v4())
                    .push_bind(tenant_id)
                    .push_bind(&event.producer)
                    .push_bind(event.event_id)
                    .push_bind(&event.event_type)
                    .push_bind(event.schema_version)
                    .push_bind(digest.to_vec())
                    .push("'received'");
            },
        );
        inbox_insert.push(" ON CONFLICT (tenant_id,producer_id,event_id) DO NOTHING");
        inbox_insert.build().execute(&mut *tx).await?;

        let producer = &events[0].producer;
        let source_system = &events[0].source_system;
        if events
            .iter()
            .any(|event| event.producer != *producer || event.source_system != *source_system)
        {
            return Err(BillingError::InvalidCommand);
        }

        let event_ids = events
            .iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>();
        let inbox_rows = sqlx::query(
            "SELECT id,event_id,event_type,schema_version,payload_sha256,disposition
             FROM platform_billing.inbox_events
             WHERE tenant_id=$1 AND producer_id=$2 AND event_id=ANY($3)",
        )
        .bind(tenant_id)
        .bind(producer)
        .bind(&event_ids)
        .fetch_all(&mut *tx)
        .await?;
        let mut inbox_by_event = HashMap::with_capacity(inbox_rows.len());
        for row in inbox_rows {
            inbox_by_event.insert(
                row.try_get::<Uuid, _>("event_id")?,
                (
                    row.try_get::<Uuid, _>("id")?,
                    row.try_get::<String, _>("event_type")?,
                    row.try_get::<i16, _>("schema_version")?,
                    row.try_get::<Vec<u8>, _>("payload_sha256")?,
                    row.try_get::<String, _>("disposition")?,
                ),
            );
        }

        let mut inbox_ids = Vec::with_capacity(events.len());
        let mut active = Vec::with_capacity(events.len());
        let mut dispositions = Vec::with_capacity(events.len());
        for (event, digest) in events.iter().zip(digests.iter()) {
            let (id, event_type, schema_version, stored_digest, disposition) = inbox_by_event
                .get(&event.event_id)
                .ok_or(BillingError::InvalidCommand)?;
            if event_type != &event.event_type
                || *schema_version != event.schema_version
                || stored_digest.as_slice() != digest.as_slice()
            {
                return Err(BillingError::InboxIdempotencyConflict);
            }
            inbox_ids.push(*id);
            if disposition == "processed" {
                dispositions.push(UsageBatchDisposition::DuplicateProcessed);
            } else {
                dispositions.push(UsageBatchDisposition::Processed);
                active.push(event);
            }
        }

        if !active.is_empty() {
            let mut usage_insert = QueryBuilder::<Postgres>::new(
                "INSERT INTO platform_billing.usage_events
                 (id,tenant_id,meter_key,source_event_id,source_system,quantity,unit,occurred_at,attributes) "
            );
            usage_insert.push_values(active.iter(), |mut row, event| {
                row.push_bind(Uuid::new_v4())
                    .push_bind(tenant_id)
                    .push_bind(&event.meter_key)
                    .push_bind(&event.source_event_id)
                    .push_bind(&event.source_system)
                    .push_bind(event.quantity)
                    .push_bind(&event.unit)
                    .push_bind(event.occurred_at)
                    .push_bind(&event.attributes);
            });
            usage_insert.push(" ON CONFLICT (tenant_id,source_system,source_event_id) DO NOTHING");
            usage_insert.build().execute(&mut *tx).await?;

            let source_event_ids = active
                .iter()
                .map(|event| event.source_event_id.clone())
                .collect::<Vec<_>>();
            let usage_rows = sqlx::query(
                "SELECT source_event_id,meter_key,quantity,unit,occurred_at,attributes
                 FROM platform_billing.usage_events
                 WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=ANY($3)",
            )
            .bind(tenant_id)
            .bind(source_system)
            .bind(&source_event_ids)
            .fetch_all(&mut *tx)
            .await?;
            let mut usage_by_source = HashMap::with_capacity(usage_rows.len());
            for row in usage_rows {
                usage_by_source.insert(
                    row.try_get::<String, _>("source_event_id")?,
                    (
                        row.try_get::<String, _>("meter_key")?,
                        row.try_get::<i64, _>("quantity")?,
                        row.try_get::<String, _>("unit")?,
                        row.try_get::<DateTime<Utc>, _>("occurred_at")?,
                        row.try_get::<Value, _>("attributes")?,
                    ),
                );
            }
            for event in &active {
                let (meter_key, quantity, unit, occurred_at, attributes) = usage_by_source
                    .get(&event.source_event_id)
                    .ok_or(BillingError::InvalidCommand)?;
                if meter_key != &event.meter_key
                    || *quantity != event.quantity
                    || unit != &event.unit
                    || *occurred_at != event.occurred_at
                    || attributes != &event.attributes
                {
                    return Err(BillingError::IdempotencyConflict);
                }
            }
            let mut mark = QueryBuilder::<Postgres>::new(
                "UPDATE platform_billing.inbox_events SET disposition='processed',
                 processed_at=COALESCE(processed_at,now()) WHERE tenant_id=",
            );
            mark.push_bind(tenant_id)
                .push(" AND id = ANY(")
                .push_bind(inbox_ids)
                .push(") AND disposition='received'");
            mark.build().execute(&mut *tx).await?;
        }

        histogram!("platform_billing_usage_sql_seconds")
            .record(sql_started.elapsed().as_secs_f64());
        let commit_started = Instant::now();
        tx.commit().await?;
        histogram!("platform_billing_usage_commit_seconds")
            .record(commit_started.elapsed().as_secs_f64());
        Ok(dispositions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn event(tenant_id: Uuid) -> UsageBatchEvent {
        let now = Utc::now();
        UsageBatchEvent {
            tenant_id,
            producer: "entitlements_quotas".to_owned(),
            event_id: Uuid::new_v4(),
            event_type: "billing.usage.recorded".to_owned(),
            schema_version: 1,
            payload: serde_json::json!({"event":"usage"}),
            source_system: "entitlements_quotas".to_owned(),
            source_event_id: Uuid::new_v4().to_string(),
            meter_key: "api.requests".to_owned(),
            quantity: 1,
            unit: "request".to_owned(),
            occurred_at: now,
            attributes: serde_json::json!({"environment":"test"}),
        }
    }

    #[tokio::test]
    async fn rejects_mixed_tenant_batch_before_opening_transaction() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
            .expect("lazy pool configuration is valid");
        let service = BillingService::new(pool);
        let tenant_a = Uuid::new_v4();
        let tenant_b = Uuid::new_v4();
        let events = [event(tenant_a), event(tenant_b)];

        let result = service
            .record_usage_batch_for_tenant(tenant_a, &events)
            .await;
        assert!(matches!(result, Err(BillingError::InvalidCommand)));
    }

    #[tokio::test]
    async fn rejects_batches_over_the_hard_memory_bound_before_database_access() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
            .expect("lazy pool configuration is valid");
        let service = BillingService::new(pool);
        let tenant_id = Uuid::new_v4();
        let event = event(tenant_id);
        let events = vec![event; 513];

        let result = service
            .record_usage_batch_for_tenant(tenant_id, &events)
            .await;
        assert!(matches!(result, Err(BillingError::InvalidCommand)));
    }
}
