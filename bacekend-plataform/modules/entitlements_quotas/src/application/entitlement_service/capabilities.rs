use uuid::Uuid;

use super::{EntitlementService, EntitlementServiceError, support::set_operator_context};
use crate::domain::entitlement::{Capability, EntitlementError};

impl EntitlementService {
    pub async fn set_capabilities(
        &self,
        actor: Uuid,
        price_version: Uuid,
        capabilities: &[Capability],
        correlation_id: &str,
    ) -> Result<u32, EntitlementServiceError> {
        if capabilities.is_empty() || capabilities.len() > 100 {
            return Err(EntitlementError::InvalidCapability.into());
        }
        let mut seen = std::collections::HashSet::new();
        for capability in capabilities {
            capability.validate()?;
            if !seen.insert(capability.key.as_str()) {
                return Err(EntitlementError::InvalidCapability.into());
            }
        }
        let mut tx = self.pool.begin().await?;
        set_operator_context(&mut tx, actor).await?;
        let existing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entitlements_quotas.plan_capabilities WHERE price_version_id=$1)")
            .bind(price_version).fetch_one(&mut *tx).await?;
        if existing {
            return Err(EntitlementError::InvalidCapability.into());
        }
        for capability in capabilities {
            sqlx::query("INSERT INTO entitlements_quotas.plan_capabilities (price_version_id,capability_key,enabled,limit_value,unit,overage_policy,environment,quota_period,created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(price_version).bind(&capability.key).bind(capability.enabled).bind(capability.limit_value)
                .bind(&capability.unit).bind(&capability.overage_policy).bind(&capability.environment)
                .bind(&capability.quota_period).bind(actor).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO entitlements_quotas.audit_events (event_id,actor_id,tenant_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,NULL,'entitlement.capabilities.configured',$3,'completed',$4,$5,$6)")
            .bind(Uuid::new_v4()).bind(actor).bind(price_version).bind(correlation_id)
            .bind(payvia_observability::current_trace_id())
            .bind(serde_json::json!({"capability_count": capabilities.len()}))
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO entitlements_quotas.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'entitlement.capabilities.configured',1,'platform',$3,$4,$5)")
            .bind(Uuid::new_v4()).bind(price_version)
            .bind(serde_json::json!({"price_version_id": price_version, "capability_count": capabilities.len()}))
            .bind(payvia_observability::current_traceparent())
            .bind(correlation_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(capabilities.len() as u32)
    }
}
