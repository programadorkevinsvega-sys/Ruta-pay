mod database;
mod operator;
mod redis;
mod servers;

use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use tracing_subscriber::prelude::*;

use crate::{
    identity_access_acceptance_context::AcceptanceContext,
    identity_access_acceptance_support::EventCapture,
};

pub(super) async fn initialize() -> AcceptanceContext {
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    let captured = EventCapture::default();
    let captured_events = captured.0.clone();
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("identity-acceptance")))
        .with(captured)
        .try_init()
        .expect("identity acceptance subscriber initializes");

    let database = database::prepare().await;
    let operator = operator::bootstrap(&database.runtime_pool).await;
    let (redis_client, redis_manager) = redis::prepare(
        &database.redis_url,
        &database.runtime_pool,
        &operator.email,
        &operator.password,
    )
    .await;
    let servers = servers::start(
        database.runtime_pool.clone(),
        redis_client,
        redis_manager.clone(),
    )
    .await;

    AcceptanceContext {
        provider,
        captured_events,
        admin_pool: database.admin_pool,
        runtime_role: database.runtime_role,
        database_cleanup: database.database_cleanup,
        migration_pool: database.migration_pool,
        runtime_pool: database.runtime_pool,
        operator_email: operator.email,
        operator_password: operator.password,
        operator_id: operator.id,
        redis_manager,
        grpc_addr: servers.grpc_addr,
        grpc_task: servers.grpc_task,
        grpc_stop: servers.grpc_stop,
        authorization_client: servers.authorization_client,
        realtime_acceptance_client: servers.realtime_acceptance_client,
        http_addr: servers.http_addr,
        origin: servers.origin,
        http_task: servers.http_task,
        http_stop: servers.http_stop,
        default_headers: servers.default_headers,
        test_source_ip: servers.test_source_ip,
        client: servers.client,
        email: servers.email,
        password: servers.password,
        state: Default::default(),
    }
}
