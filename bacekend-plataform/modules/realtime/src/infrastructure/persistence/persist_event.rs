use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::{EventEnvelope, JournalError};

pub async fn persist_event(pool: &PgPool, envelope: &EventEnvelope) -> Result<bool, JournalError> {
    if envelope.schema_version != 1
        || envelope.correlation_id.trim().is_empty()
        || envelope.correlation_id.len() > 128
        || !matches!(
            envelope.audience.as_str(),
            "tenant" | "tenant_and_platform" | "platform_operator"
        )
        || envelope.aggregate_version < 1
        || !matches!(
            envelope.event_type.as_str(),
            "tenant.registered"
                | "tenant.status.changed"
                | "membership.changed"
                | "api_key.status.changed"
                | "identity.session.revoked"
                | "identity.tenant_user.registered"
                | "identity.operator.bootstrap.completed"
                | "catalog.plan.created"
                | "catalog.price_version.published"
                | "entitlement.capabilities.configured"
                | "tenant.plan.assigned"
                | "quota.consumed"
        )
    {
        return Err(JournalError::InvalidEnvelope);
    }
    let tenant_id = envelope
        .payload
        .get("tenant_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    if matches!(envelope.audience.as_str(), "tenant" | "tenant_and_platform") != tenant_id.is_some()
    {
        return Err(JournalError::InvalidEnvelope);
    }
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT set_config('app.realtime_audience', $1, true), set_config('app.realtime_tenant_id', $2, true)")
        .bind(if envelope.audience == "tenant" { "tenant" } else { "platform" })
        .bind(tenant_id.map_or_else(String::new, |tenant_id| tenant_id.to_string()))
        .execute(&mut *transaction)
        .await?;
    let already_recorded = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM realtime.event_journal WHERE event_id=$1)",
    )
    .bind(envelope.event_id)
    .fetch_one(&mut *transaction)
    .await?;
    if already_recorded {
        transaction.commit().await?;
        return Ok(false);
    }
    let tenant_cursor = if matches!(envelope.audience.as_str(), "tenant" | "tenant_and_platform") {
        let tenant_id = tenant_id.ok_or(JournalError::InvalidEnvelope)?;
        Some(advance_scope_cursor(&mut transaction, &format!("tenant:{tenant_id}")).await?)
    } else {
        None
    };
    let platform_cursor = if matches!(
        envelope.audience.as_str(),
        "tenant_and_platform" | "platform_operator"
    ) {
        Some(advance_scope_cursor(&mut transaction, "platform").await?)
    } else {
        None
    };
    let inserted = sqlx::query(
        "INSERT INTO realtime.event_journal (event_id,event_type,schema_version,aggregate_id,aggregate_version,audience,tenant_id,tenant_cursor,platform_cursor,correlation_id,payload,occurred_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(envelope.event_id)
    .bind(&envelope.event_type)
    .bind(envelope.schema_version)
    .bind(envelope.aggregate_id)
    .bind(envelope.aggregate_version)
    .bind(&envelope.audience)
    .bind(tenant_id)
    .bind(tenant_cursor)
    .bind(platform_cursor)
    .bind(&envelope.correlation_id)
    .bind(&envelope.payload)
    .bind(envelope.occurred_at)
    .execute(&mut *transaction)
    .await?
    .rows_affected() == 1;
    if !inserted {
        transaction.rollback().await?;
        return Ok(false);
    }
    transaction.commit().await?;
    Ok(true)
}

async fn advance_scope_cursor(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope_key: &str,
) -> Result<i64, JournalError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO realtime.scope_cursors (scope_key,cursor) VALUES ($1,1) \
         ON CONFLICT (scope_key) DO UPDATE SET cursor=realtime.scope_cursors.cursor+1, updated_at=now() \
         RETURNING cursor",
    )
    .bind(scope_key)
    .fetch_one(&mut **transaction)
    .await
    .map_err(JournalError::Database)
}
