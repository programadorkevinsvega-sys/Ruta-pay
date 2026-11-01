use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let event_message = ctx.state.stream.as_ref().expect("outbox stream is initialized")
    .get_last_raw_message_by_subject(&ctx.state.registration_subject)
    .await
    .expect("JetStream durably stores tenant registration event");
ctx.state.registration_event_payload = event_message.payload.to_vec();
let event_envelope: serde_json::Value = serde_json::from_slice(&event_message.payload)
    .expect("published event uses the versioned JSON envelope");
assert_eq!(
    event_envelope["event_id"],
    ctx.state.registration_event_id.to_string()
);
assert!(
    !String::from_utf8_lossy(&event_message.payload).contains(&ctx.state.legal_identification_number)
);
let membership_created = platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
    &ctx.runtime_pool,
    &event_message.payload,
)
.await
.expect("real consumer provisions owner membership from the durable event");
assert!(membership_created);
let owner_membership: bool = sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM identity_access.tenant_memberships WHERE tenant_id=$1 AND user_id=$2 AND role='tenant_owner' AND status='active')",
)
.bind(ctx.state.created_tenant_id)
.bind(ctx.state.registered_user_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("owner membership is persisted in its owning module");
assert!(owner_membership);
let duplicate_membership = platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
    &ctx.runtime_pool,
    &event_message.payload,
)
.await
.expect("redelivery is handled idempotently");
assert!(
    !duplicate_membership,
    "redelivered event does not duplicate membership"
);
let memberships_response = ctx.client
    .get(format!("http://{http_addr}/v1/identity/memberships"))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-correlation-id", "corr_memberships_read")
    .send()
    .await
    .expect("tenant membership query reaches identity module through BFF");
assert_eq!(memberships_response.status(), StatusCode::OK);
let memberships_body: serde_json::Value = memberships_response
    .json()
    .await
    .expect("membership response is safe JSON");
assert_eq!(
    memberships_body["memberships"][0]["tenant_id"],
    ctx.state.created_tenant_id.to_string()
);
assert_eq!(memberships_body["memberships"][0]["role"], "tenant_owner");
}
