use crate::http::{
    handlers::{catalog, health_live, health_ready, identity, openapi, tenant},
    middleware::request_diagnostics,
};
use axum::{
    Router, middleware,
    routing::{delete, get, post},
};
use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient,
    entitlements_quotas::entitlements_quotas_client::EntitlementsQuotasClient,
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
    pub catalog_client: CatalogPricingClient<tonic::transport::Channel>,
    pub entitlements_client: EntitlementsQuotasClient<tonic::transport::Channel>,
}

pub fn router(
    grpc_client: RuntimeProbeClient<tonic::transport::Channel>,
    identity_client: IdentityAccessClient<tonic::transport::Channel>,
    tenant_client: TenantRegistryClient<tonic::transport::Channel>,
    realtime_client: RealtimeGatewayClient<tonic::transport::Channel>,
    catalog_client: CatalogPricingClient<tonic::transport::Channel>,
    entitlements_client: EntitlementsQuotasClient<tonic::transport::Channel>,
) -> Router {
    Router::new()
        .route("/health/live", get(health_live::handle))
        .route("/health/ready", get(health_ready::handle))
        .route("/openapi.json", get(openapi::handle))
        .route("/v1/identity/register", post(identity::register))
        .route("/v1/platform/catalog/plans", get(catalog::list_plans).post(catalog::create_plan))
        .route("/v1/platform/catalog/plans/{plan_id}/prices", post(catalog::publish_price))
        .route("/v1/platform/entitlements/capabilities", post(catalog::set_capabilities))
        .route("/v1/platform/entitlements/assignments", post(catalog::assign_plan))
        .route("/v1/tenants/{tenant_id}/entitlements", get(catalog::list_entitlements))
        .route("/v1/tenants/{tenant_id}/entitlements/{capability_key}", get(catalog::get_entitlement))
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
            catalog_client,
            entitlements_client,
        }))
        .layer(middleware::from_fn(request_diagnostics::handle))
}
