use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::domain::entitlement::{Capability, Entitlement, EntitlementError, validate_consumption};

#[derive(Clone)]
pub struct EntitlementService {
    pool: PgPool,
}

pub struct ConsumeQuotaCommand<'a> {
    pub actor: Uuid,
    pub tenant: Uuid,
    pub key: &'a str,
    pub environment: &'a str,
    pub quantity: i64,
    pub idempotency_key: &'a str,
    pub now: DateTime<Utc>,
    pub correlation_id: &'a str,
}

impl EntitlementService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

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
        sqlx::query("INSERT INTO entitlements_quotas.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'entitlement.capabilities.configured',1,'platform',$3,NULL,$4)")
            .bind(Uuid::new_v4()).bind(price_version)
            .bind(serde_json::json!({"price_version_id": price_version, "capability_count": capabilities.len()}))
            .bind(correlation_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(capabilities.len() as u32)
    }

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
        let consumed: i64 = sqlx::query_scalar("SELECT COALESCE(sum(quantity),0)::bigint FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1 AND capability_key=$2 AND period_start=$3 AND period_end=$4")
            .bind(tenant).bind(key).bind(period_start).bind(period_end).fetch_one(&mut *tx).await?;
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
        let existing = sqlx::query("SELECT capability_key,quantity,environment FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1 AND idempotency_key=$2")
            .bind(tenant).bind(idempotency_key).fetch_optional(&mut *tx).await?;
        if let Some(row) = existing {
            let old_key: String = row.try_get("capability_key")?;
            let old_quantity: i64 = row.try_get("quantity")?;
            let old_environment: String = row.try_get("environment")?;
            if old_key != key || old_quantity != quantity || old_environment != environment {
                return Err(EntitlementError::IdempotencyConflict.into());
            }
            tx.commit().await?;
            return Ok((
                self.evaluate(actor, tenant, key, environment, now).await?,
                true,
            ));
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
        let consumed: i64 = sqlx::query_scalar("SELECT COALESCE(sum(quantity),0)::bigint FROM entitlements_quotas.usage_ledger WHERE tenant_id=$1 AND capability_key=$2 AND period_start=$3 AND period_end=$4")
            .bind(tenant).bind(key).bind(period_start).bind(period_end).fetch_one(&mut *tx).await?;
        let env_allowed = allowed_environment == "all" || allowed_environment == environment;
        if !enabled
            || !env_allowed
            || limit.is_some_and(|max| consumed.saturating_add(quantity) > max) && policy == "deny"
        {
            return Err(EntitlementError::QuotaExceeded.into());
        }
        let usage_id = Uuid::new_v4();
        sqlx::query("INSERT INTO entitlements_quotas.usage_ledger (id,tenant_id,price_version_id,capability_key,environment,idempotency_key,quantity,period_start,period_end) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(usage_id).bind(tenant).bind(price_version).bind(key).bind(environment).bind(idempotency_key).bind(quantity).bind(period_start).bind(period_end).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO entitlements_quotas.audit_events (event_id,actor_id,tenant_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,$2,$3,'quota.consumed',$4,'completed',$5,$6,$7)")
            .bind(Uuid::new_v4()).bind(actor).bind(tenant).bind(usage_id).bind(correlation_id)
            .bind(payvia_observability::current_trace_id())
            .bind(serde_json::json!({"capability_key": key, "quantity": quantity, "environment": environment, "idempotency_key": idempotency_key}))
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO entitlements_quotas.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,traceparent,correlation_id) VALUES ($1,$2,1,'quota.consumed',1,'tenant',$3,NULL,$4)")
            .bind(Uuid::new_v4()).bind(usage_id)
            .bind(serde_json::json!({"tenant_id": tenant, "usage_id": usage_id, "capability_key": key, "quantity": quantity, "period_start": period_start, "period_end": period_end}))
            .bind(correlation_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok((
            self.evaluate(actor, tenant, key, environment, now).await?,
            false,
        ))
    }
}

struct EntitlementSnapshot {
    tenant_id: Uuid,
    price_version_id: Uuid,
    capability_key: String,
    enabled: bool,
    limit_value: Option<i64>,
    overage_policy: String,
    allowed_environment: String,
}

impl EntitlementSnapshot {
    fn build(
        self,
        environment: &str,
        consumed: i64,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Entitlement {
        let environment_allowed =
            self.allowed_environment == "all" || self.allowed_environment == environment;
        let allowed = self.enabled
            && environment_allowed
            && (self.limit_value.is_none()
                || self.overage_policy == "allow"
                || consumed < self.limit_value.unwrap_or_default());
        let reason = if !self.enabled {
            "capability_disabled"
        } else if !environment_allowed {
            "environment_restricted"
        } else if !allowed {
            "quota_exhausted"
        } else {
            "allowed"
        };
        Entitlement {
            tenant_id: self.tenant_id,
            price_version_id: self.price_version_id,
            capability_key: self.capability_key,
            enabled: self.enabled,
            limit_value: self.limit_value,
            consumed,
            remaining: self
                .limit_value
                .map(|value| value.saturating_sub(consumed).max(0)),
            allowed,
            reason: reason.to_owned(),
            environment: environment.to_owned(),
            period_start,
            period_end,
        }
    }
}

fn period_bounds(
    at: DateTime<Utc>,
    period: &str,
) -> Result<(DateTime<Utc>, DateTime<Utc>), EntitlementError> {
    let date = at.date_naive();
    let start = match period {
        "day" => date,
        "week" => date - chrono::Duration::days(i64::from(date.weekday().num_days_from_monday())),
        "month" => NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
            .ok_or(EntitlementError::InvalidCapability)?,
        "year" => {
            NaiveDate::from_ymd_opt(date.year(), 1, 1).ok_or(EntitlementError::InvalidCapability)?
        }
        _ => return Err(EntitlementError::InvalidCapability),
    };
    let end = match period {
        "day" => start + chrono::Duration::days(1),
        "week" => start + chrono::Duration::days(7),
        "month" => start + chrono::Months::new(1),
        "year" => start + chrono::Months::new(12),
        _ => return Err(EntitlementError::InvalidCapability),
    };
    let start = Utc.from_utc_datetime(
        &start
            .and_hms_opt(0, 0, 0)
            .ok_or(EntitlementError::InvalidCapability)?,
    );
    let end = Utc.from_utc_datetime(
        &end.and_hms_opt(0, 0, 0)
            .ok_or(EntitlementError::InvalidCapability)?,
    );
    Ok((start, end))
}

fn validate_environment(environment: &str) -> Result<(), EntitlementError> {
    if matches!(environment, "test" | "live") {
        Ok(())
    } else {
        Err(EntitlementError::InvalidCapability)
    }
}
async fn set_operator_context(
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
async fn set_tenant_context(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: Uuid,
    actor: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true),set_config('app.actor_id',$2,true)")
        .bind(tenant.to_string())
        .bind(actor.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum EntitlementServiceError {
    #[error(transparent)]
    Domain(#[from] EntitlementError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
