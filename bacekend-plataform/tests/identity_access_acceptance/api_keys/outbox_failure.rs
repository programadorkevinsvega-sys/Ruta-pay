use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let api_key_outbox_failure = ctx.client
    .post(format!(
        "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
    ))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.csrf_cookie)
    .header("x-correlation-id", "corr_api_key_outbox_failure")
    .json(&serde_json::json!({
        "name": "Rollback API key",
        "environment": "test",
        "scopes": ["payment_intents.create"]
    }))
    .send()
    .await
    .expect("API key Outbox failure receives an HTTP response");
assert_eq!(
    api_key_outbox_failure.status(),
    StatusCode::INTERNAL_SERVER_ERROR
);
let api_keys_after_outbox_failure: i64 =
    sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("API key count after Outbox failure is queryable");
assert_eq!(
    api_keys_after_outbox_failure, api_keys_before_outbox_failure,
    "API key creation must roll back when its Outbox write fails"
);
let failed_api_key_audit_rows: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.audit_events WHERE correlation_id='corr_api_key_outbox_failure'",
)
.fetch_one(&ctx.migration_pool)
.await
.expect("failed API key transaction leaves no audit row");
assert_eq!(failed_api_key_audit_rows, 0);
sqlx::raw_sql(
    "DROP TRIGGER reject_test_api_key_outbox_event ON identity_access.outbox_events;          DROP FUNCTION identity_access.reject_test_api_key_outbox_event();",
)
.execute(&ctx.migration_pool)
.await
.expect("API key failure trigger is removed");}
