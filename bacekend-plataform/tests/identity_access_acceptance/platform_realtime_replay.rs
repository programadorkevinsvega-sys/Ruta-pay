use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use redis::aio::ConnectionManager;
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;

pub struct PlatformRealtimeReplayContext<'a> {
    pub stream: &'a jetstream::stream::Stream,
    pub event_topology: &'a EventTopology,
    pub runtime_pool: &'a PgPool,
    pub realtime_redis: &'a mut ConnectionManager,
    pub client: &'a Client,
    pub http_addr: SocketAddr,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
}

pub async fn verify(context: PlatformRealtimeReplayContext<'_>) {
    let PlatformRealtimeReplayContext {
        stream, event_topology, runtime_pool, realtime_redis, client, http_addr,
        operator_session_cookie, operator_csrf_cookie,
    } = context;
    let status_event_message = stream
        .get_last_raw_message_by_subject(&format!(
            "{}.tenant.status.changed",
            event_topology.subject_prefix
        ))
        .await
        .expect("JetStream durably stores administrative status events");
    platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
        runtime_pool,
        realtime_redis,
        &status_event_message.payload,
    )
    .await
    .expect("status transition is journaled for realtime replay");
    let platform_replay = client
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
}
