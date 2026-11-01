use crate::identity_access_acceptance_context::AcceptanceContext;
use platform_contracts::realtime::SubscribeEventsRequest;
use reqwest::{header, StatusCode};
use uuid::Uuid;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let invalid_realtime_scope = ctx.client
    .get(format!(
        "http://{http_addr}/v1/realtime/events?audience=unknown&cursor=0"
    ))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .send()
    .await
    .expect("unsupported realtime audience receives an HTTP response");
assert_eq!(invalid_realtime_scope.status(), StatusCode::BAD_REQUEST);
let malformed_realtime_tenant = ctx.client
    .get(format!(
        "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id=not-a-uuid&cursor=0"
    ))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .send()
    .await
    .expect("malformed realtime tenant scope receives an HTTP response");
assert_eq!(malformed_realtime_tenant.status(), StatusCode::BAD_REQUEST);
 let foreign_tenant_replay = ctx.client
    .get(format!(
        "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id={}&cursor=0",
        Uuid::new_v4()
    ))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .send()
    .await
    .expect("tenant realtime audience cannot cross tenant boundary");
assert_eq!(foreign_tenant_replay.status(), StatusCode::FORBIDDEN);
let mut realtime_redis = ctx.redis_manager.clone();
let status_event_message = ctx.state.stream.as_ref().expect("outbox stream is initialized")
    .get_last_raw_message_by_subject(&format!(
        "{}.tenant.status.changed",
        ctx.state.event_topology.as_ref().expect("event topology is initialized").subject_prefix
    ))
    .await
    .expect("JetStream durably stores administrative status events");
platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
    &ctx.runtime_pool,
    &mut realtime_redis,
    &status_event_message.payload,
)
.await
.expect("status transition is journaled for realtime replay");
let platform_replay = ctx.client
    .get(format!("http://{http_addr}/v1/realtime/events?audience=platform&cursor=0"))
    .header(
        header::COOKIE,
        format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
    )
    .header("x-correlation-id", "corr_realtime_platform_replay")
    .send()
    .await
    .expect("platform operator recovers administrative events through authorized BFF");
assert_eq!(platform_replay.status(), StatusCode::OK);
let platform_replay_body: serde_json::Value = platform_replay
    .json()
    .await
    .expect("platform event replay is safe JSON");
assert!(
    platform_replay_body["events"]
        .as_array()
        .is_some_and(|events| {
            events
                .iter()
                .any(|event| event["event_type"] == "tenant.status.changed")
        })
);
// Keep an authenticated realtime subscription open while the operator
// session is revoked. The stream must re-check authorization and terminate.
let mut realtime_subscription_client = ctx.realtime_acceptance_client.clone();
let realtime_subscription = realtime_subscription_client
    .subscribe_events(SubscribeEventsRequest {
        session_token: ctx.state.operator_session_cookie.clone(),
        csrf_token: ctx.state.operator_csrf_cookie.clone(),
        audience: "platform".to_owned(),
        tenant_id: String::new(),
        after_cursor: 0,
        correlation_id: "corr_realtime_revocation_subscription".to_owned(),
    })
    .await
    .expect("authorized operator can subscribe to platform realtime")
    .into_inner();
 let operator_logout = ctx.client
    .delete(format!("http://{http_addr}/v1/platform/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!(
            "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
        ),
    )
    .header("x-csrf-token", &ctx.state.operator_csrf_cookie)
    .header("x-correlation-id", "corr_operator_logout")
    .send()
    .await
    .expect("operator logout response arrives");
assert_eq!(operator_logout.status(), StatusCode::NO_CONTENT);
let mut revoked_subscription = realtime_subscription;
let realtime_revocation_status = tokio::time::timeout(Duration::from_secs(20), async {
    loop {
        match revoked_subscription.message().await {
            Err(status) => break status,
            Ok(Some(_event)) => continue,
            Ok(None) => panic!("realtime subscription must report authorization revocation"),
        }
    }
})
.await
.expect("realtime subscription revalidates operator authorization within 20 seconds");
assert_eq!(
    realtime_revocation_status.code(),
    tonic::Code::Unauthenticated
);
 let operator_logout_audited: bool = sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'identity.session.revoked' AND correlation_id = 'corr_operator_logout')",
)
.bind(ctx.operator_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("operator session revocation audit is queryable");
assert!(
    operator_logout_audited,
    "operator session revocation is auditable"
);
let session_revocation_event: bool = sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM identity_access.outbox_events WHERE event_type = 'identity.session.revoked' AND audience = 'platform_operator')",
)
.fetch_one(&ctx.migration_pool)
.await
.expect("operator session revocation event is queryable");
assert!(
    session_revocation_event,
    "operator session revocation reaches durable outbox"
);
let operator_replay = ctx.client
    .get(format!("http://{http_addr}/v1/platform/identity/session"))
    .header(
        header::COOKIE,
        format!(
            "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
        ),
    )
    .send()
    .await
    .expect("revoked operator session replay is rejected");
assert_eq!(operator_replay.status(), StatusCode::UNAUTHORIZED);
 // The preceding logout deliberately revoked the first operator session.
// Create a fresh session for the role/permission matrix that follows.}
