use sqlx::Row;
use uuid::Uuid;

use super::{
    ConsumeQuotaCommand, EntitlementService, EntitlementServiceError,
    result_snapshot::EntitlementResultSnapshot,
    snapshot::EntitlementSnapshot,
    support::{period_bounds, set_tenant_context, validate_environment},
};
use crate::domain::entitlement::{Entitlement, EntitlementError, validate_consumption};

impl EntitlementService {
    pub async fn consume(
        &self,
        command: ConsumeQuotaCommand<'_>,
    ) -> Result<(Entitlement, bool), EntitlementServiceError> {
        let ConsumeQuotaCommand {
            actor,
            tenant,
            key,
            environment,
            quantity,
            idempotency_key,
            now,
            correlation_id,
        } = command;
        validate_consumption(quantity, idempotency_key)?;
        validate_environment(environment)?;
        let mut tx = self.pool.begin().await?;
        set_tenant_context(&mut tx, tenant, actor).await?;
        let idempotency_lock = format!("usage-idempotency:{tenant}:{idempotency_key}");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(idempotency_lock)
            .execute(&mut *tx)
            .await?;
        let existing = sqlx::query("SELECT capability_key,quantity,environment,result_snapshot FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1 AND idempotency_key=$2")
            .bind(tenant).bind(idempotency_key).fetch_optional(&mut *tx).await?;
        if let Some(row) = existing {
            let old_key: String = row.try_get("capability_key")?;
            let old_quantity: i64 = row.try_get("quantity")?;
            let old_environment: String = row.try_get("environment")?;
            let result_snapshot: Option<serde_json::Value> = row.try_get("result_snapshot")?;
            if old_key != key || old_quantity != quantity || old_environment != environment {
                return Err(EntitlementError::IdempotencyConflict.into());
            }
            let snapshot = result_snapshot.ok_or(EntitlementServiceError::IdempotencyResultUnavailable)?;
            let original_result =
                EntitlementResultSnapshot::decode(snapshot, tenant, key, environment)?;
            tx.commit().await?;
            return Ok((original_result, true));
        }
        let capability = sqlx::query("SELECT a.price_version_id,c.enabled,c.limit_value,c.overage_policy,c.environment,c.quota_period FROM entitlements_quotas.tenant_plan_assignments a JOIN entitlements_quotas.plan_capabilities c ON c.price_version_id=a.price_version_id WHERE a.tenant_id=$1 AND a.effective_from <= $2 AND (a.effective_until IS NULL OR a.effective_until > $2) AND c.capability_key=$3 FOR SHARE OF a")
            .bind(tenant).bind(now).bind(key).fetch_optional(&mut *tx).await?.ok_or(EntitlementError::CapabilityUnavailable)?;
        let price_version: Uuid = capability.try_get("price_version_id")?;
        let enabled: bool = capability.try_get("enabled")?;
        let limit: Option<i64> = capability.try_get("limit_value")?;
        let policy: String = capability.try_get("overage_policy")?;
        let allowed_environment: String = capability.try_get("environment")?;
        let quota_period: String = capability.try_get("quota_period")?;
        let (period_start, period_end) = period_bounds(now, &quota_period)?;
        let lock_key = format!("{tenant}:{key}:{environment}:{}", period_start.timestamp());
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;
        let consumed: i64 = sqlx::query_scalar("SELECT COALESCE(sum(quantity),0)::bigint FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1 AND capability_key=$2 AND environment=$3 AND period_start=$4 AND period_end=$5")
            .bind(tenant).bind(key).bind(environment).bind(period_start).bind(period_end).fetch_one(&mut *tx).await?;
        let env_allowed = allowed_environment == "all" || allowed_environment == environment;
        if !enabled
            || !env_allowed
            || limit.is_some_and(|max| consumed.saturating_add(quantity) > max) && policy == "deny"
        {
            return Err(EntitlementError::QuotaExceeded.into());
        }
        let consumed_after = consumed
            .checked_add(quantity)
            .ok_or(EntitlementError::InvalidQuantity)?;
        let result = EntitlementSnapshot {
            tenant_id: tenant,
            price_version_id: price_version,
            capability_key: key.to_owned(),
            enabled,
            limit_value: limit,
            overage_policy: policy.clone(),
            allowed_environment: allowed_environment.clone(),
        }
        .build(environment, consumed_after, period_start, period_end);
        let result_snapshot = EntitlementResultSnapshot::encode(&result)?;
        let usage_id = Uuid::new_v4();
        sqlx::query("INSERT INTO entitlements_quotas.usage_ledger (id,tenant_id,price_version_id,capability_key,environment,idempotency_key,quantity,period_start,period_end,result_snapshot) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(usage_id).bind(tenant).bind(price_version).bind(key).bind(environment).bind(idempotency_key).bind(quantity).bind(period_start).bind(period_end).bind(result_snapshot).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO entitlements_quotas.audit_events (event_id,actor_id,tenant_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'quota.consumed',$4,'completed',$5,$6,$7)")
            .bind(Uuid::new_v4()).bind(actor).bind(tenant).bind(usage_id).bind(correlation_id)
            .bind(payvia_observability::current_trace_id())
            .bind(serde_json::json!({"capability_key": key, "quantity": quantity, "environment": environment, "idempotency_key": idempotency_key}))
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO entitlements_quotas.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'quota.consumed',1,'tenant',$3,$4,$5)")
            .bind(Uuid::new_v4()).bind(usage_id)
            .bind(serde_json::json!({"tenant_id": tenant, "usage_id": usage_id, "capability_key": key, "quantity": quantity, "period_start": period_start, "period_end": period_end}))
            .bind(payvia_observability::current_traceparent())
            .bind(correlation_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok((result, false))
    }
}
