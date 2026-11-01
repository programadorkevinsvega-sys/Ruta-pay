use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let tenant_list = ctx.client
    .get(format!("http://{http_addr}/v1/platform/tenants?query=Acceptance&limit=10&offset=0"))
    .header(
        header::COOKIE,
        format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
    )
    .header("x-correlation-id", "corr_tenant_admin_list")
    .send()
    .await
    .expect("operator can list tenants through BFF and gRPC");
assert_eq!(tenant_list.status(), StatusCode::OK);
let tenant_list_body: serde_json::Value =
    tenant_list.json().await.expect("tenant list is JSON");
assert_eq!(tenant_list_body["total"], 1);
sqlx::raw_sql(
    "CREATE OR REPLACE FUNCTION tenant_registry.reject_test_status_outbox_event() RETURNS trigger \
     LANGUAGE plpgsql AS $tenant_status$ BEGIN \
     IF NEW.correlation_id = 'corr_tenant_status_outbox_failure' THEN \
         RAISE EXCEPTION 'injected tenant status Outbox failure'; \
     END IF; RETURN NEW; END $tenant_status$; \
     CREATE TRIGGER reject_test_status_outbox_event BEFORE INSERT ON tenant_registry.outbox_events \
     FOR EACH ROW EXECUTE FUNCTION tenant_registry.reject_test_status_outbox_event();",
)
.execute(&ctx.migration_pool)
.await
.expect("tenant status Outbox failure trigger is installed");
let tenant_before_status_outbox_failure: (String, i64) =
    sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
        .bind(ctx.state.created_tenant_id)
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("tenant state before status Outbox failure is queryable");
let status_outbox_failure = ctx.client
    .post(format!(
        "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
    ))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.operator_csrf_cookie)
    .header("x-correlation-id", "corr_tenant_status_outbox_failure")
    .json(&serde_json::json!({
        "status": "active",
        "reason": "Injected Outbox failure must roll back",
        "expected_version": tenant_before_status_outbox_failure.1
    }))
    .send()
    .await
    .expect("tenant status Outbox failure receives an HTTP response");
assert_eq!(
    status_outbox_failure.status(),
    StatusCode::INTERNAL_SERVER_ERROR
);
let tenant_after_status_outbox_failure: (String, i64) =
    sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
        .bind(ctx.state.created_tenant_id)
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("tenant state after status Outbox failure is queryable");
assert_eq!(
    tenant_after_status_outbox_failure, tenant_before_status_outbox_failure,
    "status and version must roll back when the Outbox write fails"
);
let failed_status_history_rows: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM tenant_registry.status_history WHERE tenant_id=$1 AND version=$2",
)
.bind(ctx.state.created_tenant_id)
.bind(tenant_before_status_outbox_failure.1 + 1)
.fetch_one(&ctx.migration_pool)
.await
.expect("failed status Outbox write leaves no history row");
assert_eq!(failed_status_history_rows, 0);
sqlx::raw_sql(
    "DROP TRIGGER reject_test_status_outbox_event ON tenant_registry.outbox_events;          DROP FUNCTION tenant_registry.reject_test_status_outbox_event();",
)
.execute(&ctx.migration_pool)
.await
.expect("tenant status failure trigger is removed");
let tenant_admin_status = ctx.client
    .post(format!("http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.operator_csrf_cookie)
    .header("x-correlation-id", "corr_tenant_admin_activate")
    .json(&serde_json::json!({
        "status": "active",
        "reason": "Acceptance verification",
        "expected_version": tenant_body["status_version"]
    }))
    .send()
    .await
    .expect("authorized operator can activate tenant");
assert_eq!(tenant_admin_status.status(), StatusCode::OK);
let tenant_admin_status_body: serde_json::Value = tenant_admin_status
    .json()
    .await
    .expect("status change returns the tenant state");
assert_eq!(tenant_admin_status_body["status"], "active");
assert_eq!(tenant_admin_status_body["status_version"], 2);
let tenant_status_outbox: bool = sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.status.changed' AND correlation_id='corr_tenant_admin_activate')",
)
.bind(ctx.state.created_tenant_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("tenant status mutation has a durable outbox event");
assert!(tenant_status_outbox);}
