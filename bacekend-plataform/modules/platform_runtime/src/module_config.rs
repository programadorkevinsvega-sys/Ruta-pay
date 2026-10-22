use crate::module_public_api::RuntimeProbeApi;
use platform_contracts::identity_access::identity_access_client::IdentityAccessClient as IdentityAccessGrpcClient;
use platform_contracts::{
    identity_access::identity_access_server::IdentityAccessServer,
    realtime::realtime_gateway_server::RealtimeGatewayServer,
    runtime::runtime_probe_server::RuntimeProbeServer,
    tenant_registry::tenant_registry_server::TenantRegistryServer,
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

pub async fn serve<F>(
    listener: TcpListener,
    pool: sqlx::PgPool,
    redis: ConnectionManager,
    redis_client: redis::Client,
    identity_client: IdentityAccessGrpcClient<tonic::transport::Channel>,
    shutdown: F,
) -> Result<(), tonic::transport::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    let identity_port = Arc::new(IdentityAccessGrpcAdapter::new(identity_client.clone()));
    let tenant_service = TenantService::new(pool.clone(), identity_port);
    Server::builder()
        .add_service(RuntimeProbeServer::new(RuntimeProbeApi))
        .add_service(IdentityAccessServer::new(IdentityAccessApi::new(
            IdentityService::new(pool.clone(), redis),
        )))
        .add_service(TenantRegistryServer::new(TenantRegistryApi::new(
            tenant_service,
        )))
        .add_service(RealtimeGatewayServer::new(RealtimeGatewayApi::new(
            pool.clone(),
            redis_client,
            identity_client,
        )))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
}
