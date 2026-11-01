use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
    .expect("tenant-isolation transaction completes");
 let mut realtime_redis = ctx.redis_manager.clone();
let first_journal_insert =
    platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
        &ctx.runtime_pool,
        &mut realtime_redis,
        &ctx.state.registration_event_payload,
    )
    .await
    .expect("realtime consumer persists the event journal and publishes its Redis signal");
assert!(first_journal_insert);
let journal_payload: String =
    sqlx::query_scalar("SELECT payload::text FROM realtime.event_journal WHERE event_id=$1")
        .bind(ctx.state.registration_event_id)
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("realtime journal contains the registered tenant event");
assert!(!journal_payload.contains(&ctx.state.legal_identification_number));
let duplicate_journal_insert =
    platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
        &ctx.runtime_pool,
        &mut realtime_redis,
        &event_message.payload,
    )
    .await
    .expect("redelivered JetStream event is safely deduplicated");
assert!(!duplicate_journal_insert);
let journal_event_count: i64 =
    sqlx::query_scalar("SELECT count(*) FROM realtime.event_journal WHERE event_id=$1")
        .bind(ctx.state.registration_event_id)
        .fetch_one(&ctx.migration_pool)
        .await
        .expect("journal stores the event id only once");
assert_eq!(journal_event_count, 1);
let replay_url = format!(
    "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id={created_tenant_id}"
);
let tenant_replay = ctx.client
    .get(format!("{replay_url}&cursor=0"))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-correlation-id", "corr_realtime_tenant_replay")
    .send()
    .await
    .expect("tenant can recover realtime journal by authorized cursor");
assert_eq!(tenant_replay.status(), StatusCode::OK);
let replay_body: serde_json::Value = tenant_replay
    .json()
    .await
    .expect("authorized event replay is safe JSON");
assert!(replay_body["events"].as_array().is_some_and(|events| {
    events
        .iter()
        .any(|event| event["event_type"] == "tenant.registered")
}));
ctx.state.ctx.state.replay_cursor = replay_body["next_cursor"]
    .as_u64()
    .expect("cursor is unsigned");
let cursor_replay = ctx.client
    .get(format!("{replay_url}&cursor={replay_cursor}"))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .send()
    .await
    .expect("replay resumes at the acknowledged cursor");
let cursor_replay_body: serde_json::Value = cursor_replay
    .json()
    .await
    .expect("cursor replay is safe JSON");
assert_eq!(
    cursor_replay_body["events"].as_array().map(Vec::len),
    Some(0)
);
 // A cursor ahead of the durable scope cursor is not an empty page: it is
// an invalid checkpoint and must be rejected so clients do not skip events.
let future_cursor = ctx.client
    .get(format!(
        "{replay_url}&cursor={}",
        ctx.state.replay_cursor.saturating_add(100)
    ))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-correlation-id", "corr_realtime_future_cursor")
    .send()
    .await
    .expect("future realtime cursor receives a classified response");
assert_eq!(future_cursor.status(), StatusCode::RANGE_NOT_SATISFIABLE);
 for invalid_cursor in [
    "-1",
    "not-a-number",
    "9223372036854775808",
    "18446744073709551616",
] {
    let invalid_cursor_response = ctx.client
        .get(format!("{replay_url}&cursor={invalid_cursor}"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("malformed or negative realtime cursor receives an HTTP response");
    assert_eq!(
        invalid_cursor_response.status(),
        StatusCode::BAD_REQUEST,
        "invalid cursor {invalid_cursor} must be rejected at the HTTP boundary"
    );
}}
