use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgPool, Row, postgres::PgRow};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
pub struct EventEnvelope {
    pub event_id: Uuid,
    pub event_type: String,
    pub schema_version: i16,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub audience: String,
    pub correlation_id: String,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JournalEvent {
    pub cursor: i64,
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub audience: String,
    pub tenant_id: Option<Uuid>,
    pub correlation_id: String,
    pub payload: Value,
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("integration event envelope is invalid")]
    InvalidEnvelope,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("realtime signal publication failed")]
    Redis,
    #[error("journal event has an invalid persisted format")]
    InvalidPersistedEvent,
    #[error("realtime cursor is outside the retained scope")]
    CursorOutOfRange,
    #[error("realtime journal contains a cursor gap")]
    CursorGap,
}

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

pub async fn replay(
    pool: &PgPool,
    audience: &str,
    tenant_id: Option<Uuid>,
    after_cursor: i64,
    limit: i64,
) -> Result<Vec<JournalEvent>, JournalError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT set_config('app.realtime_audience', $1, true), set_config('app.realtime_tenant_id', $2, true)")
        .bind(audience)
        .bind(tenant_id.map_or_else(String::new, |tenant_id| tenant_id.to_string()))
        .execute(&mut *transaction)
        .await?;
    if after_cursor < 0 {
        return Err(JournalError::CursorOutOfRange);
    }
    let scope_key = match audience {
        "tenant" => format!("tenant:{}", tenant_id.ok_or(JournalError::InvalidEnvelope)?),
        "platform" => "platform".to_owned(),
        _ => return Err(JournalError::InvalidEnvelope),
    };
    let current_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor FROM realtime.scope_cursors WHERE scope_key=$1",
    )
    .bind(scope_key)
    .fetch_optional(&mut *transaction)
    .await?
    .unwrap_or(0);
    if after_cursor > current_cursor {
        return Err(JournalError::CursorOutOfRange);
    }

    let rows = match audience {
        "tenant" => sqlx::query(
            "SELECT tenant_cursor AS cursor,event_id,event_type,aggregate_id,aggregate_version,audience,tenant_id,correlation_id,payload FROM realtime.event_journal \
             WHERE tenant_id=$1 AND audience IN ('tenant','tenant_and_platform') AND tenant_cursor>$2 ORDER BY tenant_cursor LIMIT $3",
        )
        .bind(tenant_id.ok_or(JournalError::InvalidEnvelope)?)
        .bind(after_cursor)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await?,
        "platform" => sqlx::query(
            "SELECT platform_cursor AS cursor,event_id,event_type,aggregate_id,aggregate_version,audience,tenant_id,correlation_id,payload FROM realtime.event_journal \
             WHERE audience IN ('tenant_and_platform','platform_operator') AND platform_cursor>$1 ORDER BY platform_cursor LIMIT $2",
        )
        .bind(after_cursor)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await?,
        _ => return Err(JournalError::InvalidEnvelope),
    };
    let events = rows.iter().map(map_event).collect::<Result<Vec<_>, _>>()?;
    let mut expected_cursor = after_cursor.saturating_add(1);
    for event in &events {
        if event.cursor != expected_cursor {
            return Err(JournalError::CursorGap);
        }
        expected_cursor = event.cursor.saturating_add(1);
    }
    transaction.commit().await?;
    Ok(events)
}

fn map_event(row: &PgRow) -> Result<JournalEvent, JournalError> {
    Ok(JournalEvent {
        cursor: row
            .try_get("cursor")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        event_id: row
            .try_get("event_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        event_type: row
            .try_get("event_type")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        aggregate_id: row
            .try_get("aggregate_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        aggregate_version: row
            .try_get("aggregate_version")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        audience: row
            .try_get("audience")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        correlation_id: row
            .try_get("correlation_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        payload: row
            .try_get("payload")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
    })
}

pub const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
    table_name: std::borrow::Cow::Borrowed("realtime._sqlx_migrations"),
    create_schemas: std::borrow::Cow::Borrowed(&[std::borrow::Cow::Borrowed("realtime")]),
    ..sqlx::migrate!("infrastructure/persistence/migrations")
};
