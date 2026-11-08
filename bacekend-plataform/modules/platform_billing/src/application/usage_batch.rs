use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row, Postgres};
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
    DuplicateUsage,
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
        if events.is_empty() || events.len() > 512 || events.iter().any(|event| {
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
        }) {
            return Err(BillingError::InvalidCommand);
        }

        let digests = events.iter().map(|event| {
            serde_json::to_vec(&event.payload)
                .map(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)))
                .map_err(|_| BillingError::InvalidCommand)
        }).collect::<Result<Vec<_>, _>>()?;
        let mut tx = self.pool.begin().await?;
        set_tenant_context(&mut tx, tenant_id).await?;

        let mut inbox_insert = QueryBuilder::<Postgres>::new(
            "INSERT INTO platform_billing.inbox_events              (id,tenant_id,producer_id,event_id,event_type,schema_version,payload_sha256,disposition) "
        );
        inbox_insert.push_values(events.iter().zip(digests.iter()), |mut row, (event, digest)| {
            row.push_bind(Uuid::new_v4())
                .push_bind(tenant_id)
                .push_bind(&event.producer)
                .push_bind(event.event_id)
                .push_bind(&event.event_type)
                .push_bind(event.schema_version)
                .push_bind(digest.to_vec())
                .push("'received'");
        });
        inbox_insert.push(" ON CONFLICT (tenant_id,producer_id,event_id) DO NOTHING");
        inbox_insert.build().execute(&mut *tx).await?;

        let mut inbox_ids = Vec::with_capacity(events.len());
        let mut active = Vec::with_capacity(events.len());
        let mut dispositions = Vec::with_capacity(events.len());
        for (event, digest) in events.iter().zip(digests.iter()) {
            let row = sqlx::query(
                "SELECT id,event_type,schema_version,payload_sha256,disposition                  FROM platform_billing.inbox_events                  WHERE tenant_id=$1 AND producer_id=$2 AND event_id=$3"
            )
            .bind(tenant_id).bind(&event.producer).bind(event.event_id)
            .fetch_one(&mut *tx).await?;
            let same = row.try_get::<String,_>("event_type")? == event.event_type
                && row.try_get::<i16,_>("schema_version")? == event.schema_version
                && row.try_get::<Vec<u8>,_>("payload_sha256")? == *digest;
            if !same { return Err(BillingError::InboxIdempotencyConflict); }
            let id: Uuid = row.try_get("id")?;
            inbox_ids.push(id);
            if row.try_get::<String,_>("disposition")? == "processed" {
                dispositions.push(UsageBatchDisposition::DuplicateProcessed);
            } else {
                dispositions.push(UsageBatchDisposition::Processed);
                active.push(event);
            }
        }

        if !active.is_empty() {
            let mut usage_insert = QueryBuilder::<Postgres>::new(
                "INSERT INTO platform_billing.usage_events                  (id,tenant_id,meter_key,source_event_id,source_system,quantity,unit,occurred_at,attributes) "
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

            for event in &active {
                let row = sqlx::query(
                    "SELECT meter_key,quantity,unit,occurred_at,attributes                      FROM platform_billing.usage_events                      WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=$3"
                )
                .bind(tenant_id).bind(&event.source_system).bind(&event.source_event_id)
                .fetch_one(&mut *tx).await?;
                let same = row.try_get::<String,_>("meter_key")? == event.meter_key
                    && row.try_get::<i64,_>("quantity")? == event.quantity
                    && row.try_get::<String,_>("unit")? == event.unit
                    && row.try_get::<DateTime<Utc>,_>("occurred_at")? == event.occurred_at
                    && row.try_get::<Value,_>("attributes")? == event.attributes;
                if !same { return Err(BillingError::IdempotencyConflict); }
            }
            let mut mark = QueryBuilder::<Postgres>::new(
                "UPDATE platform_billing.inbox_events SET disposition='processed',                  processed_at=COALESCE(processed_at,now()) WHERE tenant_id="
            );
            mark.push_bind(tenant_id).push(" AND id = ANY(")
                .push_bind(inbox_ids).push(") AND disposition='received'");
            mark.build().execute(&mut *tx).await?;
        }

        tx.commit().await?;
        Ok(dispositions)
    }
}
