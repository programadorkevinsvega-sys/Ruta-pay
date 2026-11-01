use super::*;
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct TenantStatusRaceContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub operator_id: Uuid,
    pub created_tenant_id: Uuid,
    pub current_status_version: i64,
}

pub async fn verify(context: TenantStatusRaceContext<'_>) {
    let TenantStatusRaceContext {
        client,
        migration_pool,
        http_addr,
        origin,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
        current_status_version,
    } = context;
    // Two lifecycle mutations using the same expected version must serialize:
    // one commits, the other receives a version conflict regardless of which
    // state transition wins the row lock.
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(migration_pool)
        .await
        .expect("platform owner role is restored for concurrent lifecycle acceptance");
    let race_status_url =
        format!("http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status");
    let race_cookie = format!(
        "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
    );
    let (suspend_race, close_race) = tokio::join!(
        client
            .post(&race_status_url)
            .header(header::ORIGIN, origin)
            .header(header::COOKIE, &race_cookie)
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_status_race_suspend")
            .json(&serde_json::json!({
                "status": "suspended",
                "reason": "Concurrent optimistic-lock suspension",
                "expected_version": current_status_version
            }))
            .send(),
        client
            .post(&race_status_url)
            .header(header::ORIGIN, origin)
            .header(header::COOKIE, &race_cookie)
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_status_race_close")
            .json(&serde_json::json!({
                "status": "closing",
                "reason": "Concurrent optimistic-lock close",
                "expected_version": current_status_version
            }))
            .send()
    );
    let suspend_race = suspend_race.expect("concurrent suspension response arrives");
    let close_race = close_race.expect("concurrent close response arrives");
    let mut race_statuses = vec![suspend_race.status(), close_race.status()];
    race_statuses.sort();
    assert_eq!(
        race_statuses,
        vec![StatusCode::OK, StatusCode::CONFLICT],
        "exactly one concurrent tenant status mutation may commit"
    );
    let race_conflict = if suspend_race.status() == StatusCode::CONFLICT {
        suspend_race
    } else {
        close_race
    };
    let race_conflict_body: serde_json::Value = race_conflict
        .json()
        .await
        .expect("optimistic-lock race returns a safe conflict envelope");
    assert_eq!(
        race_conflict_body["error"]["code"], "tenant_state_conflict",
        "the losing request must be classified as a stale-version conflict"
    );
    let race_final_state: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(migration_pool)
            .await
            .expect("concurrent lifecycle final state is queryable");
    assert!(
        matches!(race_final_state.0.as_str(), "suspended" | "closing"),
        "one of the two requested transitions must win"
    );
    assert_eq!(race_final_state.1, current_status_version + 1);
    let race_history_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.status_history WHERE tenant_id=$1 AND version=$2",
    )
    .bind(created_tenant_id)
    .bind(current_status_version + 1)
    .fetch_one(migration_pool)
    .await
    .expect("single concurrent status-history row is queryable");
    assert_eq!(race_history_count, 1);
    let race_outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND aggregate_version=$2 AND event_type='tenant.status.changed'",
    )
    .bind(created_tenant_id)
    .bind(current_status_version + 1)
    .fetch_one(migration_pool)
    .await
    .expect("single concurrent status Outbox event is queryable");
    assert_eq!(race_outbox_count, 1);
}
