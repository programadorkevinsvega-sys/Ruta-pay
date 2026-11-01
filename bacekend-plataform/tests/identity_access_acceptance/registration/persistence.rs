use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};
use uuid::Uuid;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let register_response = ctx.client
    .post(format!("http://{http_addr}/v1/identity/register"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_identity_register")
    .json(&serde_json::json!({"email": ctx.email, "password": ctx.password}))
    .send()
    .await
    .expect("HTTP registration response arrives");
assert_eq!(register_response.status(), StatusCode::CREATED);
 let registration: serde_json::Value = register_response
    .json()
    .await
    .expect("registration response contains its resource identifier");
ctx.state.ctx.state.registered_user_id = Uuid::parse_str(
    registration["user_id"]
        .as_str()
        .expect("registration user id is present"),
)
.expect("registration user id is valid");
 // Force the Outbox insert to fail after the identity INSERT. The transaction
// must roll back the identity and audit writes rather than leave partial state.
sqlx::raw_sql(
    "CREATE OR REPLACE FUNCTION identity_access.reject_test_outbox_event() RETURNS trigger \
     LANGUAGE plpgsql AS $phase2$ BEGIN \
     IF NEW.correlation_id = 'corr_force_identity_outbox_failure' THEN \
       RAISE EXCEPTION 'forced acceptance failure'; \
     END IF; RETURN NEW; END $phase2$; \
     CREATE TRIGGER reject_test_outbox_event BEFORE INSERT ON identity_access.outbox_events \
     FOR EACH ROW EXECUTE FUNCTION identity_access.reject_test_outbox_event();",
)
.execute(&ctx.migration_pool)
.await
.expect("temporary failure trigger is installed in the isolated acceptance database");
let rollback_email = format!("rollback_{}@example.test", Uuid::new_v4().simple());
let rollback_response = ctx.client
    .post(format!("http://{http_addr}/v1/identity/register"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_force_identity_outbox_failure")
    .json(&serde_json::json!({
        "email": rollback_email.clone(),
        "password": format!("Rollback-password-{}!", Uuid::new_v4().simple())
    }))
    .send()
    .await
    .expect("forced Outbox failure returns an HTTP response");
assert_eq!(
    rollback_response.status(),
    StatusCode::INTERNAL_SERVER_ERROR
);
let rollback_identity_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
)
.bind(rollback_email.to_ascii_lowercase())
.fetch_one(&ctx.migration_pool)
.await
.expect("rollback identity count is queryable");
assert_eq!(rollback_identity_count, 0, "identity insert must roll back");
let rollback_audit_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.audit_events WHERE correlation_id='corr_force_identity_outbox_failure'",
)
.fetch_one(&ctx.migration_pool)
.await
.expect("rollback audit count is queryable");
assert_eq!(rollback_audit_count, 0, "audit insert must not be partial");
let rollback_outbox_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.outbox_events WHERE correlation_id='corr_force_identity_outbox_failure'",
)
.fetch_one(&ctx.migration_pool)
.await
.expect("rollback Outbox count is queryable");
assert_eq!(
    rollback_outbox_count, 0,
    "Outbox insert must not be partial"
);
sqlx::raw_sql(
    "DROP TRIGGER reject_test_outbox_event ON identity_access.outbox_events; \
     DROP FUNCTION identity_access.reject_test_outbox_event();",
)
.execute(&ctx.migration_pool)
.await
.expect("temporary failure trigger is removed");
 // Duplicate normalized email must be rejected at the HTTP boundary without
// creating a second identity or a second durable registration event.
let duplicate_registration = ctx.client
    .post(format!("http://{http_addr}/v1/identity/register"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_identity_duplicate_email")
    .json(&serde_json::json!({
        "email": format!(" {} ", ctx.email.to_ascii_uppercase()),
        "password": ctx.password.clone()
    }))
    .send()
    .await
    .expect("normalized duplicate email registration receives a classified response");
assert_eq!(duplicate_registration.status(), StatusCode::CONFLICT);
let duplicate_envelope: serde_json::Value = duplicate_registration
    .json()
    .await
    .expect("duplicate registration returns a safe error envelope");
assert_eq!(duplicate_envelope["error"]["code"], "identity_conflict");
let duplicate_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
)
.bind(ctx.email.to_ascii_lowercase())
.fetch_one(&ctx.migration_pool)
.await
.expect("duplicate identity count is queryable");
assert_eq!(duplicate_count, 1, "normalized email remains unique");
let duplicate_event_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.outbox_events WHERE aggregate_id=$1 AND event_type='identity.tenant_user.registered'",
)
.bind(ctx.state.registered_user_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("registration event count is queryable");
assert_eq!(
    duplicate_event_count, 1,
    "duplicate registration emits no event"
);
 // Two simultaneous registrations for the same normalized email must have
// exactly one winner, even when both requests pass the rate limiter.
let concurrent_email = format!("concurrent_{}@example.test", Uuid::new_v4().simple());
let concurrent_password = format!("Concurrent-password-{}!", Uuid::new_v4().simple());
let concurrent_url = format!("http://{http_addr}/v1/identity/register");
let first_request = ctx.client
    .post(&concurrent_url)
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_identity_concurrent_first")
    .json(&serde_json::json!({
        "email": concurrent_email.clone(),
        "password": concurrent_password.clone()
    }))
    .send();
let second_request = ctx.client
    .post(&concurrent_url)
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_identity_concurrent_second")
    .json(&serde_json::json!({
        "email": concurrent_email.clone(),
        "password": concurrent_password.clone()
    }))
    .send();
let (first_result, second_result) = tokio::join!(first_request, second_request);
let first_status = first_result
    .expect("first concurrent registration response arrives")
    .status();
let second_status = second_result
    .expect("second concurrent registration response arrives")
    .status();
let mut concurrent_statuses = vec![first_status, second_status];
concurrent_statuses.sort();
assert_eq!(
    concurrent_statuses,
    vec![StatusCode::CREATED, StatusCode::CONFLICT],
    "one concurrent registration succeeds and the other conflicts"
);
let concurrent_identity_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
)
.bind(concurrent_email.to_ascii_lowercase())
.fetch_one(&ctx.migration_pool)
.await
.expect("concurrent identity count is queryable");
assert_eq!(concurrent_identity_count, 1);
 let registration_event: bool = sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM identity_access.outbox_events WHERE aggregate_id = $1 AND event_type = 'identity.tenant_user.registered')",
)
.bind(ctx.state.registered_user_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("registration outbox event is queryable");
assert!(
    registration_event,
    "registration is written to durable outbox"
);
let registration_audit: bool = sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'tenant_user.registered' AND correlation_id = 'corr_identity_register')",
)
.bind(ctx.state.registered_user_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("registration audit event is queryable");
assert!(registration_audit, "registration is auditable");
}
