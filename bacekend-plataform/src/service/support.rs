use payvia_platform_bff::common::grpc::runtime_probe_client::RuntimeProbeClient;
use platform_contracts::runtime::runtime_probe_client::RuntimeProbeClient as RuntimeProbeGrpcClient;
use std::time::Duration;
use tokio::{net::TcpListener, sync::watch};
use tracing::Instrument;

pub(super) async fn bind_listener(
    address: std::net::SocketAddr,
    component: &'static str,
    operation: &'static str,
) -> Result<TcpListener, std::io::Error> {
    TcpListener::bind(address).await.inspect_err(|error| {
        tracing::error!(
            event.name = "service.startup.bind.failed",
            component,
            operation,
            dependency = "operating_system_socket",
            error_class = "connectivity",
            error_code = error.raw_os_error().unwrap_or(-1)
        );
    })
}

pub(super) async fn serve_runtime(
    listener: TcpListener,
    dependencies: platform_runtime::module_config::RuntimeModuleDependencies,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), tonic::transport::Error> {
    let span = tracing::info_span!("grpc.server", component = "platform_runtime");
    async move {
        platform_runtime::module_config::serve(listener, dependencies, async move {
            let _ = shutdown.changed().await;
        })
        .await
    }
    .instrument(span)
    .await
}

pub(super) async fn verify_runtime_startup(
    client: RuntimeProbeGrpcClient<tonic::transport::Channel>,
) -> Result<(), tonic::Status> {
    let span = tracing::info_span!(
        "grpc.client.startup_readiness",
        correlation_id = "startup_probe"
    );
    async move {
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            RuntimeProbeClient::check(&client, "startup_probe"),
        )
        .await
        .map_err(|_| tonic::Status::deadline_exceeded("runtime startup probe timed out"))??;
        if response.status != "ready" || response.component != "platform_runtime" {
            return Err(tonic::Status::failed_precondition(
                "runtime startup probe rejected",
            ));
        }
        Ok(())
    }
    .instrument(span)
    .await
}
