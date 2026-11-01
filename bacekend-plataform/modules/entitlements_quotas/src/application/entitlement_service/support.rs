use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use uuid::Uuid;

use crate::domain::entitlement::EntitlementError;

pub(super) fn period_bounds(
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

pub(super) fn validate_environment(environment: &str) -> Result<(), EntitlementError> {
    if matches!(environment, "test" | "live") {
        Ok(())
    } else {
        Err(EntitlementError::InvalidCapability)
    }
}
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
pub(super) async fn set_tenant_context(
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
