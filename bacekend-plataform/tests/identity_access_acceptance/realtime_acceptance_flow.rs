use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use redis::aio::ConnectionManager;
use reqwest::Client;
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct RealtimeAcceptanceFlowContext<'a> {
    pub redis_manager: &'a ConnectionManager,
    pub runtime_pool: &'a PgPool,
    pub migration_pool: &'a PgPool,
    pub event_payload: &'a [u8],
    pub registration_event_id: Uuid,
    pub legal_identification_number: &'a str,
    pub client: &'a Client,
    pub http_addr: SocketAddr,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
    pub stream: &'a jetstream::stream::Stream,
    pub event_topology: &'a EventTopology,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
}

pub async fn verify(context: RealtimeAcceptanceFlowContext<'_>) -> u64 {
    let RealtimeAcceptanceFlowContext {
        redis_manager, runtime_pool, migration_pool, event_payload, registration_event_id,
        legal_identification_number, client, http_addr, session_cookie, csrf_cookie,
        created_tenant_id, stream, event_topology, operator_session_cookie,
        operator_csrf_cookie,
    } = context;
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
        super::platform_realtime_replay::PlatformRealtimeReplayContext {
            stream,
            event_topology,
            runtime_pool,
            realtime_redis: &mut realtime_redis,
            client,
            http_addr,
            operator_session_cookie,
            operator_csrf_cookie,
        },
    )
    .await;
    replay_cursor
}
