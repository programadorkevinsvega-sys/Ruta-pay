use sqlx::Transaction;
use uuid::Uuid;

pub(super) async fn set_operator_context(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('app.operator_scope','true',true),set_config('app.actor_id',$1,true)",
    )
    .bind(actor.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}
