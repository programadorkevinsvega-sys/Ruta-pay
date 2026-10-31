use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::domain::tenant::{Tenant, TenantError, TenantStatus};

#[derive(Clone, Debug)]
pub struct PendingOutboxEvent {
    pub event_id: Uuid,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub event_type: String,
    pub schema_version: i16,
    pub audience: String,
    pub payload: serde_json::Value,
    pub traceparent: Option<String>,
    pub correlation_id: String,
    pub lease_token: Uuid,
}

pub struct ChangeTenantStatusRecord<'a> {
    pub tenant_id: Uuid,
    pub actor_id: Uuid,
    pub actor_is_operator: bool,
    pub next: TenantStatus,
    pub reason: &'a str,
    pub correlation_id: &'a str,
    pub expected_version: i64,
}

pub struct CreatePendingTenantRecord<'a> {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub display_name: &'a str,
    pub slug: &'a str,
    pub identification_type: &'a str,
    pub identification_number: &'a str,
    pub correlation_id: &'a str,
}

pub async fn claim_outbox_events(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<PendingOutboxEvent>, sqlx::Error> {
    let lease_token = Uuid::new_v4();
    let rows = sqlx::query(
        "WITH candidates AS (\
            SELECT event_id FROM tenant_registry.outbox_events \
            WHERE published_at IS NULL AND available_at <= now() \
              AND (lease_until IS NULL OR lease_until < now()) \
            ORDER BY occurred_at, event_id FOR UPDATE SKIP LOCKED LIMIT $1\
         ) \
         UPDATE tenant_registry.outbox_events event \
         SET lease_token=$2, lease_until=now() + interval '30 seconds', attempts=attempts+1 \
         FROM candidates WHERE event.event_id=candidates.event_id \
         RETURNING event.event_id, event.aggregate_id, event.aggregate_version, event.event_type, \
                   event.schema_version, event.audience, event.payload, event.traceparent, event.correlation_id, event.lease_token",
    )
    .bind(limit)
    .bind(lease_token)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(PendingOutboxEvent {
                event_id: row.try_get("event_id")?,
                aggregate_id: row.try_get("aggregate_id")?,
                aggregate_version: row.try_get("aggregate_version")?,
                event_type: row.try_get("event_type")?,
                schema_version: row.try_get("schema_version")?,
                audience: row.try_get("audience")?,
                payload: row.try_get("payload")?,
                traceparent: row.try_get("traceparent")?,
                correlation_id: row.try_get("correlation_id")?,
                lease_token: row.try_get("lease_token")?,
            })
        })
        .collect()
}

pub async fn mark_outbox_published(
    pool: &PgPool,
    event_id: Uuid,
    lease_token: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE tenant_registry.outbox_events SET published_at=now(), lease_token=NULL, lease_until=NULL, last_error_code=NULL \
         WHERE event_id=$1 AND lease_token=$2 AND published_at IS NULL",
    )
    .bind(event_id)
    .bind(lease_token)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn reschedule_outbox_event(
    pool: &PgPool,
    event_id: Uuid,
    lease_token: Uuid,
    error_code: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE tenant_registry.outbox_events \
         SET available_at=now() + make_interval(secs => LEAST(300, power(2, LEAST(attempts, 8))::integer)), \
             lease_token=NULL, lease_until=NULL, last_error_code=$3 \
         WHERE event_id=$1 AND lease_token=$2 AND published_at IS NULL",
    )
    .bind(event_id)
    .bind(lease_token)
    .bind(error_code)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
    table_name: std::borrow::Cow::Borrowed("tenant_registry._sqlx_migrations"),
    create_schemas: std::borrow::Cow::Borrowed(&[std::borrow::Cow::Borrowed("tenant_registry")]),
    ..sqlx::migrate!("infrastructure/persistence/migrations")
};

pub async fn create_pending_tenant(
    pool: &PgPool,
    record: CreatePendingTenantRecord<'_>,
) -> Result<Tenant, sqlx::Error> {
    let CreatePendingTenantRecord {
        id,
        actor_id,
        display_name,
        slug,
        identification_type,
        identification_number,
        correlation_id,
    } = record;
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT set_config('app.actor_id', $1, true)")
        .bind(actor_id.to_string())
        .execute(&mut *transaction)
        .await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await?;
    let row = sqlx::query(
        "INSERT INTO tenant_registry.tenants (id, display_name, slug, legal_identification_type, legal_identification_number, status, created_by) \
         VALUES ($1, $2, $3, $4, $5, 'pending_verification', $6) \
         RETURNING id, display_name, slug, status, created_by, status_version, created_at, updated_at, closed_at",
    )
    .bind(id)
    .bind(display_name.trim())
    .bind(slug)
    .bind(identification_type)
    .bind(identification_number)
    .bind(actor_id)
    .fetch_one(&mut *transaction)
    .await?;
    let tenant =
        map_tenant(&row).map_err(|_| sqlx::Error::Protocol("invalid tenant status".to_owned()))?;
    sqlx::query(
        "INSERT INTO tenant_registry.status_history (tenant_id, version, previous_status, new_status, actor_id, reason) \
         VALUES ($1, 1, NULL, 'pending_verification', $2, 'tenant registration submitted')",
    )
    .bind(tenant.id)
    .bind(actor_id)
    .execute(&mut *transaction)
    .await?;
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant.id,
        "status": tenant.status.as_str(),
        "status_version": tenant.status_version,
        "created_by": tenant.created_by,
        "slug": tenant.slug,
    });
    sqlx::query(
        "INSERT INTO tenant_registry.outbox_events \
         (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload, correlation_id) \
         VALUES ($1, $2, 1, 'tenant.registered', 1, 'tenant_and_platform', $3, $4)",
    )
    .bind(event_id)
    .bind(tenant.id)
    .bind(payload)
    .bind(correlation_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(tenant)
}

pub async fn change_tenant_status(
    pool: &PgPool,
    record: ChangeTenantStatusRecord<'_>,
) -> Result<Tenant, TenantRepositoryError> {
    let ChangeTenantStatusRecord {
        tenant_id,
        actor_id,
        actor_is_operator,
        next,
        reason,
        correlation_id,
        expected_version,
    } = record;
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "SELECT set_config('app.operator_scope', $1, true), set_config('app.actor_id', $2, true)",
    )
    .bind(actor_is_operator.to_string())
    .bind(actor_id.to_string())
    .execute(&mut *transaction)
    .await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *transaction)
        .await?;
    let row = sqlx::query(
        "SELECT id, display_name, slug, status, created_by, status_version, created_at, updated_at, closed_at \
         FROM tenant_registry.tenants WHERE id = $1 FOR UPDATE",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(TenantRepositoryError::NotFound)?;
    let mut tenant = map_tenant(&row)?;
    Tenant::validate_transition(tenant.status, next, reason, actor_is_operator)
        .map_err(TenantRepositoryError::Domain)?;
    if tenant.status_version != expected_version {
        return Err(TenantRepositoryError::VersionConflict);
    }
    let previous = tenant.status;
    tenant.status_version += 1;
    tenant.status = next;
    tenant.updated_at = Utc::now();
    if next == TenantStatus::Closed {
        tenant.closed_at = Some(tenant.updated_at);
    }
    sqlx::query(
        "UPDATE tenant_registry.tenants \
         SET status = $2, status_version = $3, verification_reason = $4, updated_at = $5, closed_at = $6 \
         WHERE id = $1",
    )
    .bind(tenant.id)
    .bind(next.as_str())
    .bind(tenant.status_version)
    .bind(reason.trim())
    .bind(tenant.updated_at)
    .bind(tenant.closed_at)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO tenant_registry.status_history \
         (tenant_id, version, previous_status, new_status, actor_id, reason) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tenant.id)
    .bind(tenant.status_version)
    .bind(previous.as_str())
    .bind(next.as_str())
    .bind(actor_id)
    .bind(reason.trim())
    .execute(&mut *transaction)
    .await?;
    let payload = serde_json::json!({
        "tenant_id": tenant.id,
        "status": next.as_str(),
        "status_version": tenant.status_version,
        "actor_id": actor_id,
    });
    sqlx::query(
        "INSERT INTO tenant_registry.outbox_events \
         (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload, correlation_id) \
         VALUES ($1, $2, $3, 'tenant.status.changed', 1, 'tenant_and_platform', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant.id)
    .bind(tenant.status_version)
    .bind(payload)
    .bind(correlation_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(tenant)
}

pub async fn list_tenants(
    pool: &PgPool,
    actor_id: Uuid,
    query: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Tenant>, i64), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "SELECT set_config('app.operator_scope', 'true', true), set_config('app.actor_id', $1, true)",
    )
    .bind(actor_id.to_string())
    .execute(&mut *transaction)
    .await?;
    let normalized_query = query.trim().to_lowercase();
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.tenants WHERE strpos(lower(display_name), $1) > 0 OR strpos(lower(slug), $1) > 0",
    )
    .bind(&normalized_query)
    .fetch_one(&mut *transaction)
    .await?;
    let rows = sqlx::query(
        "SELECT id, display_name, slug, status, created_by, status_version, created_at, updated_at, closed_at \
         FROM tenant_registry.tenants WHERE strpos(lower(display_name), $1) > 0 OR strpos(lower(slug), $1) > 0 \
         ORDER BY created_at DESC, id LIMIT $2 OFFSET $3",
    )
    .bind(normalized_query)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *transaction)
    .await?;
    let tenants = rows
        .iter()
        .map(map_tenant)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| sqlx::Error::Protocol("invalid tenant record".to_owned()))?;
    transaction.commit().await?;
    Ok((tenants, total))
}

pub async fn get_tenant(
    pool: &PgPool,
    actor_id: Uuid,
    tenant_id: Uuid,
    operator_scope: bool,
) -> Result<Option<Tenant>, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "SELECT set_config('app.operator_scope', $1, true), set_config('app.actor_id', $2, true), set_config('app.tenant_id', $3, true)",
    )
    .bind(operator_scope.to_string())
    .bind(actor_id.to_string())
    .bind(tenant_id.to_string())
    .execute(&mut *transaction)
    .await?;
    let row = sqlx::query(
        "SELECT id, display_name, slug, status, created_by, status_version, created_at, updated_at, closed_at FROM tenant_registry.tenants WHERE id=$1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let tenant = row
        .as_ref()
        .map(map_tenant)
        .transpose()
        .map_err(|_| sqlx::Error::Protocol("invalid tenant record".to_owned()))?;
    transaction.commit().await?;
    Ok(tenant)
}

#[derive(Debug, thiserror::Error)]
pub enum TenantRepositoryError {
    #[error("tenant not found")]
    NotFound,
    #[error("tenant status version changed")]
    VersionConflict,
    #[error(transparent)]
    Domain(#[from] TenantError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

fn map_tenant(row: &sqlx::postgres::PgRow) -> Result<Tenant, TenantError> {
    let status: String = row
        .try_get("status")
        .map_err(|_| TenantError::InvalidStatus)?;
    Ok(Tenant {
        id: row.try_get("id").map_err(|_| TenantError::InvalidStatus)?,
        display_name: row
            .try_get("display_name")
            .map_err(|_| TenantError::InvalidDisplayName)?,
        slug: row.try_get("slug").map_err(|_| TenantError::InvalidSlug)?,
        status: TenantStatus::parse(&status)?,
        created_by: row
            .try_get("created_by")
            .map_err(|_| TenantError::InvalidStatus)?,
        status_version: row
            .try_get("status_version")
            .map_err(|_| TenantError::InvalidStatus)?,
        created_at: row
            .try_get::<DateTime<Utc>, _>("created_at")
            .map_err(|_| TenantError::InvalidStatus)?,
        updated_at: row
            .try_get::<DateTime<Utc>, _>("updated_at")
            .map_err(|_| TenantError::InvalidStatus)?,
        closed_at: row
            .try_get("closed_at")
            .map_err(|_| TenantError::InvalidStatus)?,
    })
}
