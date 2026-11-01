use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::entitlement::EntitlementError;
use super::{EntitlementService, EntitlementServiceError, support::set_operator_context};

impl EntitlementService {
        pub async fn assign_plan(
            &self,
            actor: Uuid,
            tenant: Uuid,
            price_version: Uuid,
            from: DateTime<Utc>,
            until: Option<DateTime<Utc>>,
            correlation_id: &str,
        ) -> Result<Uuid, EntitlementServiceError> {
            if until.is_some_and(|end| end <= from) {
                return Err(EntitlementError::PlanNotAssigned.into());
            }
            let mut tx = self.pool.begin().await?;
            set_operator_context(&mut tx, actor).await?;
            let tenant_lock = format!("tenant-plan-assignment:{tenant}");
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(tenant_lock)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE entitlements_quotas.tenant_plan_assignments SET effective_until=$2 WHERE tenant_id=$1 AND effective_from < $2 AND effective_until IS NULL")
                .bind(tenant).bind(from).execute(&mut *tx).await?;
            let overlap: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entitlements_quotas.tenant_plan_assignments WHERE tenant_id=$1 AND tstzrange(effective_from,effective_until,'[)') && tstzrange($2,$3,'[)'))")
                .bind(tenant).bind(from).bind(until).fetch_one(&mut *tx).await?;
            if overlap {
                return Err(EntitlementError::AssignmentConflict.into());
            }
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO entitlements_quotas.tenant_plan_assignments (id,tenant_id,price_version_id,effective_from,effective_until,assigned_by) VALUES ($1,$2,$3,$4,$5,$6)")
                .bind(id).bind(tenant).bind(price_version).bind(from).bind(until).bind(actor).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO entitlements_quotas.audit_events (event_id,actor_id,tenant_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'tenant.plan.assigned',$4,'completed',$5,$6,$7)")
                .bind(Uuid::new_v4()).bind(actor).bind(tenant).bind(id).bind(correlation_id)
                .bind(payvia_observability::current_trace_id())
                .bind(serde_json::json!({"price_version_id": price_version, "effective_from": from, "effective_until": until}))
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO entitlements_quotas.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'tenant.plan.assigned',1,'tenant',$3,NULL,$4)")
                .bind(Uuid::new_v4()).bind(id)
                .bind(serde_json::json!({"tenant_id": tenant, "assignment_id": id, "price_version_id": price_version, "effective_from": from, "effective_until": until}))
                .bind(correlation_id).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(id)
        }
}
