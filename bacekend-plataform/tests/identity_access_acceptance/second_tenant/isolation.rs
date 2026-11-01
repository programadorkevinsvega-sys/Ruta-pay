use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};
use uuid::Uuid;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let second_tenant_create = ctx.client
    .post(format!("http://{http_addr}/v1/tenants"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.second_csrf_cookie)
    .header("x-correlation-id", "corr_second_tenant_onboarding")
    .json(&serde_json::json!({
        "display_name": "Second acceptance tenant",
        "slug": format!("second-acceptance-{}", Uuid::new_v4().simple()),
        "legal_identification_type": "cc",
        "legal_identification_number": ctx.state.second_legal_identification_number.clone()
    }))
    .send()
    .await
    .expect("second tenant onboarding reaches its owning module");
assert_eq!(second_tenant_create.status(), StatusCode::CREATED);
let second_tenant_body: serde_json::Value = second_tenant_create
    .json()
    .await
    .expect("second tenant response is safe JSON");
ctx.state.ctx.state.second_tenant_id = Uuid::parse_str(
    second_tenant_body["tenant_id"]
        .as_str()
        .expect("second tenant identifier is present"),
)
.expect("second tenant identifier is a UUID");
let second_document: (String, String) = sqlx::query_as(
    "SELECT legal_identification_type, legal_identification_number FROM tenant_registry.tenants WHERE id=$1",
)
.bind(ctx.state.second_tenant_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("tenant registry persists the CC document");
assert_eq!(second_document.0, "cc");
assert_eq!(second_document.1, ctx.state.second_legal_identification_number);
platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
    &ctx.runtime_pool,
    &jetstream,
    &event_topology.subject_prefix,
)
.await
.expect("second tenant registration publishes through the real JetStream relay");
let second_tenant_event = ctx.state.stream.as_ref().expect("outbox stream is initialized")
    .get_last_raw_message_by_subject(&ctx.state.registration_subject)
    .await
    .expect("JetStream durably stores second tenant registration");
let second_tenant_envelope: serde_json::Value =
    serde_json::from_slice(&second_tenant_event.payload)
        .expect("second tenant event is a versioned envelope");
assert_eq!(
    second_tenant_envelope["payload"]["tenant_id"],
    ctx.state.second_tenant_id.to_string()
);
assert!(platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
    &ctx.runtime_pool,
    &second_tenant_event.payload,
)
.await
.expect("second membership is provisioned from JetStream event"));
 let cross_tenant_details = ctx.client
    .get(format!("http://{http_addr}/v1/tenants/{created_tenant_id}"))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
    )
    .header("x-correlation-id", "corr_cross_tenant_details")
    .send()
    .await
    .expect("second tenant user cannot read the first tenant");
assert_eq!(cross_tenant_details.status(), StatusCode::FORBIDDEN);
let cross_tenant_api_keys = ctx.client
    .get(format!("http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
    )
    .send()
    .await
    .expect("second tenant user cannot enumerate first tenant API keys");
assert_eq!(cross_tenant_api_keys.status(), StatusCode::FORBIDDEN);
let mut isolation_transaction = ctx.runtime_pool
    .begin()
    .await
    .expect("runtime tenant-isolation transaction begins");
sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
    .bind(ctx.state.second_tenant_id.to_string())
    .execute(&mut *isolation_transaction)
    .await
    .expect("runtime tenant context is set");
let foreign_key_count: i64 =
    sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys WHERE tenant_id=$1")
        .bind(ctx.state.created_tenant_id)
        .fetch_one(&mut *isolation_transaction)
        .await
        .expect("RLS filters API keys belonging to another tenant");
assert_eq!(foreign_key_count, 0);
isolation_transaction
    .commit()
    .await
    .expect("tenant-isolation transaction completes");
 let mut realtime_redis = redis_manager.clone();}
