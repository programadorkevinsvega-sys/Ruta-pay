use crate::module_public_api::RuntimeProbeApi;
use platform_contracts::{
    identity_access::identity_access_server::IdentityAccessServer,
    runtime::runtime_probe_server::RuntimeProbeServer,
};
use platform_identity_access::{
    application::identity_service::IdentityService,
    infrastructure::grpc::identity_access_api::IdentityAccessApi,
};
use redis::aio::ConnectionManager;
use std::future::Future;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

pub async fn serve<F>(
    listener: TcpListener,
    pool: sqlx::PgPool,
    redis: ConnectionManager,
    shutdown: F,
) -> Result<(), tonic::transport::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    Server::builder()
        .add_service(RuntimeProbeServer::new(RuntimeProbeApi))
        .add_service(IdentityAccessServer::new(IdentityAccessApi::new(
            IdentityService::new(pool, redis),
        )))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
}
