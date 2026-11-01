use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};
use uuid::Uuid;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let second_email = format!("second_tenant_{}@example.test", Uuid::new_v4().simple());
let second_password = format!("Second-tenant-password-{}!", Uuid::new_v4().simple());
let second_user_registration = ctx.client
    .post(format!("http://{http_addr}/v1/identity/register"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_second_tenant_register")
    .json(&serde_json::json!({"email": second_email, "password": second_password}))
    .send()
    .await
    .expect("second tenant user registers through the BFF");
assert_eq!(second_user_registration.status(), StatusCode::CREATED);
let second_login = ctx.client
    .post(format!("http://{http_addr}/v1/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_second_tenant_login")
    .json(&serde_json::json!({"email": second_email, "password": second_password}))
    .send()
    .await
    .expect("second tenant user logs in through the BFF");
assert_eq!(second_login.status(), StatusCode::OK);
let second_cookies: Vec<String> = second_login
    .headers()
    .get_all(header::SET_COOKIE)
    .iter()
    .filter_map(|value| value.to_str().ok().map(str::to_owned))
    .collect();
ctx.state.ctx.state.second_session_cookie = cookie_value(&second_cookies, "payvia_tenant_session");
ctx.state.ctx.state.second_csrf_cookie = cookie_value(&second_cookies, "payvia_tenant_csrf");
ctx.state.ctx.state.second_legal_identification_number = format!(
    "8{:018}",
    Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
);
let duplicate_slug_identification = format!(
    "8{:018}",
    Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
);
let conflicting_slug = ctx.client
    .post(format!("http://{http_addr}/v1/tenants"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.second_csrf_cookie)
    .header("x-correlation-id", "corr_duplicate_tenant_slug")
    .json(&serde_json::json!({
        "display_name": "Duplicate slug tenant",
        "slug": tenant_slug.clone(),
        "legal_identification_type": "cc",
        "legal_identification_number": duplicate_slug_identification
    }))
    .send()
    .await
    .expect("duplicate tenant slug returns a conflict");
assert_eq!(conflicting_slug.status(), StatusCode::CONFLICT);
let duplicate_slug_count: i64 =
    sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants WHERE slug=$1")
        .bind(&tenant_slug)
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("duplicate slug count is queryable");
assert_eq!(duplicate_slug_count, 1, "tenant slug remains unique");
let duplicate_slug_event_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
)
.bind(ctx.state.created_tenant_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("tenant registration event count is queryable");
assert_eq!(duplicate_slug_event_count, 1);
 // Concurrent tenant registration with the same slug must commit exactly
// one aggregate and one durable registration event.}
