use crate::http::{
    handlers::{health_live, health_ready, identity, openapi, tenant},
    middleware::request_diagnostics,
};
use axum::{
    Router, middleware,
    routing::{delete, get, post},
};
use platform_contracts::{
    identity_access::identity_access_client::IdentityAccessClient,
    realtime::realtime_gateway_client::RealtimeGatewayClient,
    runtime::runtime_probe_client::RuntimeProbeClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub grpc_client: RuntimeProbeClient<tonic::transport::Channel>,
    pub identity_client: IdentityAccessClient<tonic::transport::Channel>,
    pub tenant_client: TenantRegistryClient<tonic::transport::Channel>,
    pub realtime_client: RealtimeGatewayClient<tonic::transport::Channel>,
}

pub fn router(
    grpc_client: RuntimeProbeClient<tonic::transport::Channel>,
    identity_client: IdentityAccessClient<tonic::transport::Channel>,
    tenant_client: TenantRegistryClient<tonic::transport::Channel>,
    realtime_client: RealtimeGatewayClient<tonic::transport::Channel>,
) -> Router {
    Router::new()
        .route("/health/live", get(health_live::handle))
        .route("/health/ready", get(health_ready::handle))
        .route("/openapi.json", get(openapi::handle))
        .route("/v1/identity/register", post(identity::register))
        .route(
            "/v1/identity/memberships",
            get(identity::tenant_memberships),
        )
        .route("/v1/realtime/events", get(tenant::replay_events))
        .route("/v1/realtime/stream", get(tenant::websocket_events))
        .route("/v1/tenants", post(tenant::register))
        .route("/v1/tenants/{tenant_id}", get(tenant::get))
        .route("/v1/platform/tenants", get(tenant::list_for_operator))
        .route(
            "/v1/platform/tenants/{tenant_id}/status",
            post(tenant::change_status),
        )
        .route(
            "/v1/identity/session",
            get(identity::tenant_session)
                .post(identity::login)
                .delete(identity::logout),
        )
        .route(
            "/v1/platform/identity/session",
            get(identity::operator_session)
                .post(identity::operator_login)
                .delete(identity::operator_logout),
        )
        .route(
            "/v1/tenants/{tenant_id}/api-keys",
            get(identity::list_api_keys).post(identity::create_api_key),
        )
        .route(
            "/v1/tenants/{tenant_id}/api-keys/{key_id}",
            delete(identity::revoke_api_key),
        )
        .with_state(Arc::new(AppState {
            grpc_client,
            identity_client,
            tenant_client,
            realtime_client,
        }))
        .layer(middleware::from_fn(request_diagnostics::handle))
}
