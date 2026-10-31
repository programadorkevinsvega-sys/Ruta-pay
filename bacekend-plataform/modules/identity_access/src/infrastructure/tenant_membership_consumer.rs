use serde::Deserialize;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::infrastructure::persistence::provision_tenant_owner_membership;

#[derive(Deserialize)]
struct EventEnvelope {
    event_id: Uuid,
    event_type: String,
    schema_version: i16,
    correlation_id: String,
    traceparent: Option<String>,
    payload: TenantRegistered,
}

#[derive(Deserialize)]
struct TenantRegistered {
    tenant_id: Uuid,
    created_by: Uuid,
}

#[derive(Debug, Error)]
pub enum TenantMembershipConsumerError {
    #[error("tenant registration event has an unsupported envelope")]
    InvalidEnvelope,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub async fn consume_tenant_registered(
    pool: &PgPool,
    payload: &[u8],
) -> Result<bool, TenantMembershipConsumerError> {
    let event: EventEnvelope = serde_json::from_slice(payload)
        .map_err(|_| TenantMembershipConsumerError::InvalidEnvelope)?;
    if event.event_type != "tenant.registered"
        || event.schema_version != 1
        || event.correlation_id.trim().is_empty()
        || event.correlation_id.len() > 128
    {
        return Err(TenantMembershipConsumerError::InvalidEnvelope);
    }
    provision_tenant_owner_membership(
        pool,
        event.event_id,
        event.payload.tenant_id,
        event.payload.created_by,
        &event.correlation_id,
        event.traceparent.as_deref(),
    )
    .await
    .map_err(TenantMembershipConsumerError::Database)
}
