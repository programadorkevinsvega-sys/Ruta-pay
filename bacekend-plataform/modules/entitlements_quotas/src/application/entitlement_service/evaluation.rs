use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::domain::entitlement::{Entitlement, EntitlementError};
use super::{
    EntitlementService, EntitlementServiceError,
    snapshot::EntitlementSnapshot,
    support::{period_bounds, set_tenant_context, validate_environment},
};

impl EntitlementService {
        pub async fn evaluate(
            &self,
            actor: Uuid,
            tenant: Uuid,
            key: &str,
            environment: &str,
            at: DateTime<Utc>,
        ) -> Result<Entitlement, EntitlementServiceError> {
            validate_environment(environment)?;
            let mut tx = self.pool.begin().await?;
            set_tenant_context(&mut tx, tenant, actor).await?;
            let row = sqlx::query("SELECT a.price_version_id,c.enabled,c.limit_value,c.overage_policy,c.environment,c.quota_period FROM entitlements_quotas.tenant_plan_assignments a JOIN entitlements_quotas.plan_capabilities c ON c.price_version_id=a.price_version_id WHERE a.tenant_id=$1 AND a.effective_from <= $2 AND (a.effective_until IS NULL OR a.effective_until > $2) AND c.capability_key=$3")
                .bind(tenant).bind(at).bind(key).fetch_optional(&mut *tx).await?.ok_or(EntitlementError::CapabilityUnavailable)?;
            let price_version: Uuid = row.try_get("price_version_id")?;
            let enabled: bool = row.try_get("enabled")?;
            let limit: Option<i64> = row.try_get("limit_value")?;
            let policy: String = row.try_get("overage_policy")?;
            let allowed_environment: String = row.try_get("environment")?;
            let quota_period: String = row.try_get("quota_period")?;
            let (period_start, period_end) = period_bounds(at, &quota_period)?;
            let consumed: i64 = sqlx::query_scalar("SELECT COALESCE(sum(quantity),0)::bigint FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1 AND capability_key=$2 AND environment=$3 AND period_start=$4 AND period_end=$5")
                .bind(tenant).bind(key).bind(environment).bind(period_start).bind(period_end).fetch_one(&mut *tx).await?;
            tx.commit().await?;
            Ok(EntitlementSnapshot {
                tenant_id: tenant,
                price_version_id: price_version,
                capability_key: key.to_owned(),
                enabled,
                limit_value: limit,
                overage_policy: policy,
                allowed_environment,
            }
            .build(environment, consumed, period_start, period_end))
        }
    
        pub async fn list(
            &self,
            actor: Uuid,
            tenant: Uuid,
            environment: &str,
            at: DateTime<Utc>,
        ) -> Result<Vec<Entitlement>, EntitlementServiceError> {
            validate_environment(environment)?;
            let mut tx = self.pool.begin().await?;
            set_tenant_context(&mut tx, tenant, actor).await?;
            let keys = sqlx::query_scalar::<_, String>(
                "SELECT c.capability_key FROM entitlements_quotas.tenant_plan_assignments a
                 JOIN entitlements_quotas.plan_capabilities c ON c.price_version_id=a.price_version_id
                 WHERE a.tenant_id=$1 AND a.effective_from <= $2 AND (a.effective_until IS NULL OR a.effective_until > $2)
                 ORDER BY c.capability_key"
            ).bind(tenant).bind(at).fetch_all(&mut *tx).await?;
            tx.commit().await?;
            let mut entitlements = Vec::with_capacity(keys.len());
            for key in keys {
                entitlements.push(self.evaluate(actor, tenant, &key, environment, at).await?);
            }
            Ok(entitlements)
        }
}
