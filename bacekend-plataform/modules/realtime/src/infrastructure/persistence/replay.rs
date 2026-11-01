use sqlx::{PgPool, Row, postgres::PgRow};
use uuid::Uuid;

use super::{JournalError, JournalEvent};

pub async fn replay(
    pool: &PgPool,
    audience: &str,
    tenant_id: Option<Uuid>,
    after_cursor: i64,
    limit: i64,
) -> Result<Vec<JournalEvent>, JournalError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT set_config('app.realtime_audience', $1, true), set_config('app.realtime_tenant_id', $2, true)")
        .bind(audience)
        .bind(tenant_id.map_or_else(String::new, |tenant_id| tenant_id.to_string()))
        .execute(&mut *transaction)
        .await?;
    if after_cursor < 0 {
        return Err(JournalError::CursorOutOfRange);
    }
    let scope_key = match audience {
        "tenant" => format!("tenant:{}", tenant_id.ok_or(JournalError::InvalidEnvelope)?),
        "platform" => "platform".to_owned(),
        _ => return Err(JournalError::InvalidEnvelope),
    };
    let current_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor FROM realtime.scope_cursors WHERE scope_key=$1",
    )
    .bind(scope_key)
    .fetch_optional(&mut *transaction)
    .await?
    .unwrap_or(0);
    if after_cursor > current_cursor {
        return Err(JournalError::CursorOutOfRange);
    }

    let rows = match audience {
        "tenant" => sqlx::query(
            "SELECT tenant_cursor AS cursor,event_id,event_type,aggregate_id,aggregate_version,audience,tenant_id,correlation_id,payload FROM realtime.event_journal \
             WHERE tenant_id=$1 AND audience IN ('tenant','tenant_and_platform') AND tenant_cursor>$2 ORDER BY tenant_cursor LIMIT $3",
        )
        .bind(tenant_id.ok_or(JournalError::InvalidEnvelope)?)
        .bind(after_cursor)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await?,
        "platform" => sqlx::query(
            "SELECT platform_cursor AS cursor,event_id,event_type,aggregate_id,aggregate_version,audience,tenant_id,correlation_id,payload FROM realtime.event_journal \
             WHERE audience IN ('tenant_and_platform','platform_operator') AND platform_cursor>$1 ORDER BY platform_cursor LIMIT $2",
        )
        .bind(after_cursor)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await?,
        _ => return Err(JournalError::InvalidEnvelope),
    };
    let events = rows.iter().map(map_event).collect::<Result<Vec<_>, _>>()?;
    validate_cursor_sequence(
        after_cursor,
        current_cursor,
        &events,
        events.len() < limit as usize,
    )?;
    transaction.commit().await?;
    Ok(events)
}

fn validate_cursor_sequence(
    after_cursor: i64,
    current_cursor: i64,
    events: &[JournalEvent],
    page_complete: bool,
) -> Result<(), JournalError> {
    let mut expected_cursor = after_cursor.saturating_add(1);
    for event in events {
        if event.cursor != expected_cursor {
            return Err(JournalError::CursorGap);
        }
        expected_cursor = event.cursor.saturating_add(1);
    }
    if page_complete && expected_cursor <= current_cursor {
        return Err(JournalError::CursorGap);
    }
    Ok(())
}

fn map_event(row: &PgRow) -> Result<JournalEvent, JournalError> {
    Ok(JournalEvent {
        cursor: row
            .try_get("cursor")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        event_id: row
            .try_get("event_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        event_type: row
            .try_get("event_type")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        aggregate_id: row
            .try_get("aggregate_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        aggregate_version: row
            .try_get("aggregate_version")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        audience: row
            .try_get("audience")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        correlation_id: row
            .try_get("correlation_id")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
        payload: row
            .try_get("payload")
            .map_err(|_| JournalError::InvalidPersistedEvent)?,
    })
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    fn event(cursor: i64) -> JournalEvent {
        JournalEvent {
            cursor,
            event_id: Uuid::new_v4(),
            event_type: "tenant.registered".to_owned(),
            aggregate_id: Uuid::new_v4(),
            aggregate_version: 1,
            audience: "tenant".to_owned(),
            tenant_id: Some(Uuid::new_v4()),
            correlation_id: "cursor-test".to_owned(),
            payload: serde_json::json!({}),
        }
    }

    #[test]
    fn replay_accepts_a_contiguous_cursor_sequence() {
        assert!(validate_cursor_sequence(4, 7, &[event(5), event(6), event(7)], true).is_ok());
    }

    #[test]
    fn replay_rejects_a_missing_cursor_in_the_middle() {
        assert!(matches!(
            validate_cursor_sequence(4, 7, &[event(5), event(7)], true),
            Err(JournalError::CursorGap)
        ));
    }

    #[test]
    fn replay_rejects_a_gap_before_the_first_returned_event() {
        assert!(matches!(
            validate_cursor_sequence(4, 7, &[event(6)], true),
            Err(JournalError::CursorGap)
        ));
    }

    #[test]
    fn replay_rejects_a_gap_when_all_remaining_events_are_missing() {
        assert!(matches!(
            validate_cursor_sequence(4, 7, &[], true),
            Err(JournalError::CursorGap)
        ));
    }

    #[test]
    fn replay_accepts_an_empty_page_at_the_current_cursor() {
        assert!(validate_cursor_sequence(7, 7, &[], true).is_ok());
    }

    #[test]
    fn replay_allows_a_full_page_before_the_scope_cursor() {
        assert!(validate_cursor_sequence(4, 7, &[event(5), event(6)], false).is_ok());
    }
}
