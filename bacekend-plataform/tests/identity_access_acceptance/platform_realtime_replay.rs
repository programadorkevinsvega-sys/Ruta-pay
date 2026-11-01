use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use redis::aio::ConnectionManager;
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;

pub async fn verify(
    stream: &jetstream::stream::Stream,
    event_topology: &EventTopology,
    runtime_pool: &PgPool,
    realtime_redis: &mut ConnectionManager,
    client: &Client,
    http_addr: SocketAddr,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
) {
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
