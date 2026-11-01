use crate::module_public_api::RuntimeProbeApi;
use platform_catalog_pricing::{
    application::catalog_service::CatalogService,
    infrastructure::grpc::catalog_api::CatalogPricingApi,
};
use platform_contracts::identity_access::identity_access_client::IdentityAccessClient as IdentityAccessGrpcClient;
use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient,
};
use platform_contracts::{
    catalog_pricing::catalog_pricing_server::CatalogPricingServer,
    entitlements_quotas::entitlements_quotas_server::EntitlementsQuotasServer,
    identity_access::identity_access_server::IdentityAccessServer,
    realtime::realtime_gateway_server::RealtimeGatewayServer,
    runtime::runtime_probe_server::RuntimeProbeServer,
    tenant_registry::tenant_registry_server::TenantRegistryServer,
};
use platform_entitlements_quotas::{
    application::entitlement_service::EntitlementService,
    infrastructure::grpc::entitlements_api::EntitlementsQuotasApi,
};
use platform_identity_access::{
    application::identity_service::IdentityService,
    infrastructure::grpc::identity_access_api::IdentityAccessApi,
};
use platform_realtime::infrastructure::grpc::realtime_gateway_api::RealtimeGatewayApi;
use platform_tenant_registry::{
    application::tenant_service::TenantService,
    infrastructure::grpc::{
        identity_access_adapter::IdentityAccessGrpcAdapter, tenant_registry_api::TenantRegistryApi,
    },
};
use redis::aio::ConnectionManager;
use std::{future::Future, sync::Arc};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

pub struct RuntimeModuleDependencies {
    pub pool: sqlx::PgPool,
    pub redis: ConnectionManager,
    pub redis_client: redis::Client,
    pub identity_client: IdentityAccessGrpcClient<tonic::transport::Channel>,
    pub catalog_client: CatalogPricingClient<tonic::transport::Channel>,
    pub tenant_client: TenantRegistryClient<tonic::transport::Channel>,
}

pub async fn serve<F>(
    listener: TcpListener,
    dependencies: RuntimeModuleDependencies,
    shutdown: F,
) -> Result<(), tonic::transport::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    let RuntimeModuleDependencies {
        pool,
        redis,
        redis_client,
        identity_client,
        catalog_client,
        tenant_client,
    } = dependencies;
    let identity_port = Arc::new(IdentityAccessGrpcAdapter::new(identity_client.clone()));
    let tenant_service = TenantService::new(pool.clone(), identity_port);
    let catalog_api =
        CatalogPricingApi::new(CatalogService::new(pool.clone()), identity_client.clone());
    let entitlements_api = EntitlementsQuotasApi::new(
        EntitlementService::new(pool.clone()),
        identity_client.clone(),
        catalog_client,
        tenant_client,
    );
    Server::builder()
        .add_service(RuntimeProbeServer::new(RuntimeProbeApi))
        .add_service(IdentityAccessServer::new(IdentityAccessApi::new(
            IdentityService::new(pool.clone(), redis),
        )))
        .add_service(TenantRegistryServer::new(TenantRegistryApi::new(
            tenant_service,
        )))
        .add_service(CatalogPricingServer::new(catalog_api))
        .add_service(EntitlementsQuotasServer::new(entitlements_api))
        .add_service(RealtimeGatewayServer::new(RealtimeGatewayApi::new(
            pool,
            redis_client,
            identity_client,
        )))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
}
