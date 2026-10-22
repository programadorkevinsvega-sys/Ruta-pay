use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::domain::tenant::{Tenant, TenantError, TenantStatus};

pub const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
    table_name: std::borrow::Cow::Borrowed("tenant_registry._sqlx_migrations"),
    create_schemas: std::borrow::Cow::Borrowed(&[std::borrow::Cow::Borrowed("tenant_registry")]),
    ..sqlx::migrate!("infrastructure/persistence/migrations")
};

pub async fn create_pending_tenant(
    pool: &PgPool,
    id: Uuid,
    actor_id: Uuid,
    display_name: &str,
    slug: &str,
) -> Result<Tenant, sqlx::Error> {
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
        "INSERT INTO tenant_registry.tenants (id, display_name, slug, status, created_by) \
         VALUES ($1, $2, $3, 'pending_verification', $4) \
         RETURNING id, display_name, slug, status, created_by, status_version, created_at, updated_at, closed_at",
    )
    .bind(id)
    .bind(display_name.trim())
    .bind(slug)
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
         (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload) \
         VALUES ($1, $2, 1, 'tenant.registered', 1, 'tenant_and_platform', $3)",
    )
    .bind(event_id)
    .bind(tenant.id)
    .bind(payload)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(tenant)
}

pub async fn change_tenant_status(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_id: Uuid,
    actor_is_operator: bool,
    next: TenantStatus,
    reason: &str,
) -> Result<Tenant, TenantRepositoryError> {
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
         (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload) \
         VALUES ($1, $2, $3, 'tenant.status.changed', 1, 'tenant_and_platform', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant.id)
    .bind(tenant.status_version)
    .bind(payload)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(tenant)
}

#[derive(Debug, thiserror::Error)]
pub enum TenantRepositoryError {
    #[error("tenant not found")]
    NotFound,
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
