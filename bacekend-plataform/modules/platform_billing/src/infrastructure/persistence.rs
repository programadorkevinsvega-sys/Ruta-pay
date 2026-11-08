use sqlx::PgPool;

pub const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
    table_name: std::borrow::Cow::Borrowed("platform_billing._sqlx_migrations"),
    create_schemas: std::borrow::Cow::Borrowed(&[std::borrow::Cow::Borrowed("platform_billing")]),
    ..sqlx::migrate!("infrastructure/persistence/migrations")
};

pub async fn set_tenant_context(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool).await?;
    Ok(())
}
