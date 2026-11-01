use async_nats::jetstream;
use platform_contracts::realtime::realtime_gateway_client::RealtimeGatewayClient;
use platform_runtime::event_topology::EventTopology;
use redis::aio::ConnectionManager;
use reqwest::Client;
use sqlx::PgPool;
use std::net::SocketAddr;
use tonic::transport::Channel;
use uuid::Uuid;

pub async fn verify(
    redis_manager: &ConnectionManager,
    runtime_pool: &PgPool,
    migration_pool: &PgPool,
    event_payload: &[u8],
    registration_event_id: Uuid,
    legal_identification_number: &str,
    client: &Client,
    http_addr: SocketAddr,
    session_cookie: &str,
    csrf_cookie: &str,
    created_tenant_id: Uuid,
    stream: &jetstream::stream::Stream,
    event_topology: &EventTopology,
    realtime_client: &mut RealtimeGatewayClient<Channel>,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
) -> u64 {
    let mut realtime_redis = super::realtime_journal_acceptance::verify(
        redis_manager,
        runtime_pool,
        migration_pool,
        event_payload,
        registration_event_id,
        legal_identification_number,
    )
    .await;
    let replay_cursor = super::tenant_realtime_replay::verify(
        client,
        http_addr,
        session_cookie,
        csrf_cookie,
        created_tenant_id,
    )
    .await;
    super::platform_realtime_replay::verify(
        stream,
        event_topology,
        runtime_pool,
        &mut realtime_redis,
        realtime_client,
        client,
        http_addr,
        operator_session_cookie,
        operator_csrf_cookie,
    )
    .await;
    replay_cursor
}
