use uuid::Uuid;

use super::IdentityError;

pub(super) async fn set_tenant_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    actor_id: Uuid,
) -> Result<(), IdentityError> {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true),set_config('app.actor_id',$2,true)")
        .bind(tenant_id.to_string())
        .bind(actor_id.to_string())
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub(super) async fn insert_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    action: &str,
    actor_id: Uuid,
    tenant_id: Uuid,
    resource_id: Uuid,
    correlation_id: &str,
) -> Result<(), IdentityError> {
    sqlx::query("INSERT INTO identity_access.audit_events (event_id,actor_kind,actor_id,tenant_id,action,resource_id,outcome,correlation_id) VALUES ($1,'tenant_user',$2,$3,$4,$5,'completed',$6)")
        .bind(Uuid::new_v4()).bind(actor_id).bind(tenant_id).bind(action).bind(resource_id).bind(correlation_id).execute(&mut **transaction).await?;
    Ok(())
}
