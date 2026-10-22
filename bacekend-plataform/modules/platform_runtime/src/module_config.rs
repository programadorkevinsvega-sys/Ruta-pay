use crate::module_public_api::RuntimeProbeApi;
use platform_contracts::runtime::runtime_probe_server::RuntimeProbeServer;
use std::future::Future;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

pub async fn serve<F>(listener: TcpListener, shutdown: F) -> Result<(), tonic::transport::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    Server::builder()
        .add_service(RuntimeProbeServer::new(RuntimeProbeApi))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
}
