use uuid::Uuid;

use super::{CatalogService, CatalogServiceError, support::set_operator_context};

impl CatalogService {
        pub async fn create_plan(
            &self,
            actor_id: Uuid,
            slug: &str,
            name: &str,
            description: &str,
            correlation_id: &str,
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
            sqlx::query("INSERT INTO catalog_pricing.audit_events (event_id,actor_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,'catalog.plan.created',$3,'completed',$4,$5,$6)")
                .bind(Uuid::new_v4()).bind(actor_id).bind(row.0).bind(correlation_id)
                .bind(payvia_observability::current_trace_id())
                .bind(serde_json::json!({"slug": slug, "status": "draft"}))
                .execute(&mut *transaction).await?;
            sqlx::query("INSERT INTO catalog_pricing.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'catalog.plan.created',1,'platform',$3,NULL,$4)")
                .bind(Uuid::new_v4()).bind(row.0)
                .bind(serde_json::json!({"plan_id": row.0, "slug": slug, "name": name.trim()}))
                .bind(correlation_id).execute(&mut *transaction).await?;
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
        pub async fn list_plans(&self, actor_id: Uuid) -> Result<Vec<Plan>, CatalogServiceError> {
            let mut transaction = self.pool.begin().await?;
            set_operator_context(&mut transaction, actor_id).await?;
            let rows = sqlx::query_as::<_, (Uuid, String, String, String, String, DateTime<Utc>)>(
                "SELECT id,slug,name,description,status,created_at              FROM catalog_pricing.plans ORDER BY created_at,id",
            )
            .fetch_all(&mut *transaction)
            .await?;
            transaction.commit().await?;
            Ok(rows
                .into_iter()
                .map(|row| Plan {
                    id: row.0,
                    slug: row.1,
                    name: row.2,
                    description: row.3,
                    status: row.4,
                    created_at: row.5,
                })
                .collect())
        }
    }
}
