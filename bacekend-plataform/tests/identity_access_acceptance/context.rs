use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
};

use async_nats::jetstream;
use opentelemetry_sdk::trace::SdkTracerProvider;
use platform_contracts::{
    identity_access::identity_access_client::IdentityAccessClient,
    realtime::realtime_gateway_client::RealtimeGatewayClient,
};
use platform_runtime::event_topology::EventTopology;
use redis::aio::ConnectionManager;
use reqwest::{Client, header::HeaderMap};
use sqlx::PgPool;
use tokio::{sync::oneshot, task::JoinHandle};
use uuid::Uuid;

use crate::identity_access_acceptance_support::{
    AcceptanceDatabaseCleanup, EventStreamCleanup,
};

pub(super) type GrpcTask = JoinHandle<Result<(), tonic::transport::Error>>;
pub(super) type HttpTask = JoinHandle<Result<(), std::io::Error>>;

pub(super) struct AcceptanceContext {
    pub(super) provider: SdkTracerProvider,
    pub(super) captured_events: Arc<Mutex<Vec<HashMap<String, String>>>>,
    pub(super) admin_pool: PgPool,
    pub(super) runtime_role: String,
    pub(super) database_cleanup: AcceptanceDatabaseCleanup,
    pub(super) migration_pool: PgPool,
    pub(super) runtime_pool: PgPool,
    pub(super) operator_email: String,
    pub(super) operator_password: String,
    pub(super) operator_id: Uuid,
    pub(super) redis_manager: ConnectionManager,
    pub(super) grpc_addr: SocketAddr,
    pub(super) grpc_task: GrpcTask,
    pub(super) grpc_stop: oneshot::Sender<()>,
    pub(super) authorization_client: IdentityAccessClient<tonic::transport::Channel>,
    pub(super) realtime_acceptance_client: RealtimeGatewayClient<tonic::transport::Channel>,
    pub(super) http_addr: SocketAddr,
    pub(super) origin: String,
    pub(super) http_task: HttpTask,
    pub(super) http_stop: oneshot::Sender<()>,
    pub(super) default_headers: HeaderMap,
    pub(super) test_source_ip: Ipv4Addr,
    pub(super) client: Client,
    pub(super) email: String,
    pub(super) password: String,
    pub(super) state: ScenarioState,
}

#[derive(Default)]
pub(super) struct ScenarioState {
    pub(super) registered_user_id: Uuid,
    pub(super) trace_id: String,
    pub(super) traceparent: String,
    pub(super) account_key: String,
    pub(super) ip_key: String,
    pub(super) session_cookie: String,
    pub(super) csrf_cookie: String,
    pub(super) operator_session_cookie: String,
    pub(super) operator_csrf_cookie: String,
    pub(super) legal_identification_number: String,
    pub(super) tenant_slug: String,
    pub(super) created_tenant_id: Uuid,
    pub(super) jetstream: Option<jetstream::Context>,
    pub(super) event_topology: Option<EventTopology>,
    pub(super) stream: Option<jetstream::stream::Stream>,
    pub(super) event_stream_cleanup: Option<EventStreamCleanup>,
    pub(super) isolated_stream_cleanup: Option<EventStreamCleanup>,
    pub(super) registration_subject: String,
    pub(super) registration_event_id: Uuid,
    pub(super) api_key_id: Uuid,
    pub(super) api_key_secret: String,
    pub(super) second_session_cookie: String,
    pub(super) second_csrf_cookie: String,
    pub(super) second_legal_identification_number: String,
    pub(super) second_tenant_id: Uuid,
    pub(super) matrix_operator_session_cookie: String,
    pub(super) matrix_operator_csrf_cookie: String,
    pub(super) current_status_version: i64,
}
