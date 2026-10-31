use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::catalog::{CatalogError, Plan, PriceVersion};

#[derive(Clone)]
pub struct CatalogService {
    pool: PgPool,
}

impl CatalogService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_plan(
        &self,
        actor_id: Uuid,
        slug: &str,
        name: &str,
        description: &str,
    ) -> Result<Plan, CatalogServiceError> {
        Plan::validate(slug, name, description)?;
        let mut transaction = self.pool.begin().await?;
        set_operator_context(&mut transaction, actor_id).await?;
        let row = sqlx::query_as::<_, (Uuid, String, String, String, String, DateTime<Utc>)>(
            "INSERT INTO catalog_pricing.plans (id,slug,name,description,status,created_by)              VALUES ($1,$2,$3,$4,'draft',$5) RETURNING id,slug,name,description,status,created_at",
        )
        .bind(Uuid::new_v4())
        .bind(slug)
        .bind(name.trim())
        .bind(description.trim())
        .bind(actor_id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Plan {
            id: row.0,
            slug: row.1,
            name: row.2,
            description: row.3,
            status: row.4,
            created_at: row.5,
        })
    }

    pub async fn publish_price_version(
        &self,
        actor_id: Uuid,
        plan_id: Uuid,
        currency: &str,
        amount_minor: i64,
        period_unit: &str,
        interval_count: i32,
        effective_from: DateTime<Utc>,
        effective_until: Option<DateTime<Utc>>,
    ) -> Result<PriceVersion, CatalogServiceError> {
        PriceVersion::validate(
            currency,
            amount_minor,
            period_unit,
            interval_count,
            effective_from,
            effective_until,
        )?;
        let mut transaction = self.pool.begin().await?;
        set_operator_context(&mut transaction, actor_id).await?;
        sqlx::query("SELECT id FROM catalog_pricing.plans WHERE id=$1 FOR UPDATE")
            .bind(plan_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(CatalogError::PlanNotFound)?;
        let overlap: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM catalog_pricing.price_versions              WHERE plan_id=$1 AND tstzrange(effective_from,effective_until,'[)')              && tstzrange($2,$3,'[)'))",
        )
        .bind(plan_id)
        .bind(effective_from)
        .bind(effective_until)
        .fetch_one(&mut *transaction)
        .await?;
        if overlap {
            return Err(CatalogError::EffectivePeriodConflict.into());
        }
        let version: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version),0)+1 FROM catalog_pricing.price_versions WHERE plan_id=$1",
        )
        .bind(plan_id)
        .fetch_one(&mut *transaction)
        .await?;
        let row = sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
            "INSERT INTO catalog_pricing.price_versions              (id,plan_id,version,currency,amount_minor,period_unit,interval_count,effective_from,effective_until)              VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id,published_at",
        )
        .bind(Uuid::new_v4())
        .bind(plan_id)
        .bind(version)
        .bind(currency)
        .bind(amount_minor)
        .bind(period_unit)
        .bind(interval_count)
        .bind(effective_from)
        .bind(effective_until)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query("UPDATE catalog_pricing.plans SET status='active' WHERE id=$1 AND status='draft'")
            .bind(plan_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(PriceVersion {
            id: row.0,
            plan_id,
            version,
            currency: currency.to_owned(),
            amount_minor,
            period_unit: period_unit.to_owned(),
            interval_count,
            effective_from,
            effective_until,
            published_at: row.1,
        })
    }

    pub async fn get_price_version(
        &self,
        id: Uuid,
    ) -> Result<PriceVersion, CatalogServiceError> {
        let row = sqlx::query_as::<_, (Uuid, Uuid, i32, String, i64, String, i32, DateTime<Utc>, Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT id,plan_id,version,currency,amount_minor,period_unit,interval_count,effective_from,effective_until,published_at              FROM catalog_pricing.price_versions WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(CatalogError::PriceVersionNotFound)?;
        Ok(PriceVersion {
            id: row.0,
            plan_id: row.1,
            version: row.2,
            currency: row.3,
            amount_minor: row.4,
            period_unit: row.5,
            interval_count: row.6,
            effective_from: row.7,
            effective_until: row.8,
            published_at: row.9,
        })
    }

    pub async fn list_plans(&self, actor_id: Uuid) -> Result<Vec<Plan>, CatalogServiceError> {
        let mut transaction = self.pool.begin().await?;
        set_operator_context(&mut transaction, actor_id).await?;
        let rows = sqlx::query_as::<_, (Uuid, String, String, String, String, DateTime<Utc>)>(
            "SELECT id,slug,name,description,status,created_at              FROM catalog_pricing.plans ORDER BY created_at,id",
        )
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(rows.into_iter().map(|row| Plan {
            id: row.0, slug: row.1, name: row.2, description: row.3,
            status: row.4, created_at: row.5,
        }).collect())
    }
}

async fn set_operator_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('app.operator_scope','true',true),set_config('app.actor_id',$1,true)",
    )
    .bind(actor_id.to_string())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogServiceError {
    #[error(transparent)]
    Domain(#[from] CatalogError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
