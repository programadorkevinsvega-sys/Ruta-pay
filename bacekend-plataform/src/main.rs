use payvia_platform_bff::http::routes::router;
use payvia_platform_core::{config::AppConfig, metrics, telemetry};
use platform_contracts::runtime::{ReadinessRequest, runtime_probe_client::RuntimeProbeClient};
use std::time::Duration;
use tokio::{net::TcpListener, signal, sync::watch};
use tonic::transport::Endpoint;
use tracing::Instrument;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = match AppConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event.name": "service.configuration.failed",
                    "component": "platform_config",
                    "operation": "load_configuration",
                    "error_code": error.code(),
                    "error_category": "validation",
                    "variable": error.variable(),
                })
            );
            return Err(Box::new(error) as Box<dyn std::error::Error>);
        }
    };
    let telemetry = telemetry::initialize(&config).map_err(|_| {
        eprintln!(
            "{}",
            serde_json::json!({
                "event.name": "service.telemetry.initialization.failed",
                "component": "platform_telemetry",
                "operation": "initialize",
                "error_code": "telemetry_initialization_failed",
                "error_category": "configuration",
            })
        );
        std::io::Error::other("telemetry initialization failed")
    })?;
    let metrics_handle = metrics::install_recorder()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let runtime_listener = bind_listener(config.grpc_addr, "platform_runtime", "bind_grpc").await?;
    let runtime_addr = runtime_listener.local_addr()?;
    let runtime_task = tokio::spawn(serve_runtime(runtime_listener, shutdown_rx.clone()));
    let endpoint = Endpoint::from_shared(format!("http://{runtime_addr}"))?
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2));
    let grpc_client = RuntimeProbeClient::connect(endpoint).await.map_err(|status| {
        tracing::error!(event.name = "service.startup.dependency.failed", component = "platform_bff", operation = "connect_runtime", dependency = "platform_runtime", error_code = "grpc_connect_failed", error_category = "connectivity", dependency_error = %status, retryable = true);
        status
    })?;
    verify_runtime_startup(grpc_client.clone()).await.inspect_err(|status| {
        tracing::error!(event.name = "service.startup.readiness.failed", component = "platform_bff", operation = "startup_readiness", dependency = "platform_runtime", error_code = %status.code(), error_category = "dependency", retryable = status.code() == tonic::Code::Unavailable || status.code() == tonic::Code::DeadlineExceeded);
    })?;
    let metrics_listener =
        bind_listener(config.metrics_addr, "platform_metrics", "bind_metrics").await?;
    let metrics_task = tokio::spawn(metrics::serve(
        metrics_listener,
        metrics_handle,
        config.metrics_token.clone(),
        shutdown_rx.clone(),
    ));
    let http_listener = bind_listener(config.http_addr, "platform_bff", "bind_http").await?;
    tracing::info!(event.name = "service.started", service = "payvia-platform", http_addr = %http_listener.local_addr()?, grpc_addr = %runtime_addr, metrics_addr = %config.metrics_addr);
    axum::serve(http_listener, router(grpc_client))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        })
        .await?;
    runtime_task.await??;
    metrics_task.await??;
    telemetry.tracer_provider.shutdown()?;
    telemetry.logger_provider.shutdown()?;
    drop(telemetry.sentry_guard);
    Ok(())
}

async fn bind_listener(
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

async fn serve_runtime(
    listener: TcpListener,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), tonic::transport::Error> {
    let span = tracing::info_span!("grpc.server", component = "platform_runtime");
    async move {
        platform_runtime::module_config::serve(listener, async move {
            let _ = shutdown.changed().await;
        })
        .await
    }
    .instrument(span)
    .await
}

async fn verify_runtime_startup(
    mut client: RuntimeProbeClient<tonic::transport::Channel>,
) -> Result<(), tonic::Status> {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        client.check_readiness(tonic::Request::new(ReadinessRequest {
            correlation_id: "startup_probe".to_owned(),
        })),
    )
    .await
    .map_err(|_| tonic::Status::deadline_exceeded("runtime startup probe timed out"))??;
    if response.get_ref().status != "ready" || response.get_ref().component != "platform_runtime" {
        return Err(tonic::Status::failed_precondition(
            "runtime startup probe rejected",
        ));
    }
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = signal::ctrl_c().await {
        tracing::error!(event.name = "service.shutdown.signal_failed", component = "platform_runtime", error_class = "signal", error_code = %error);
    }
    tracing::info!(
        event.name = "service.shutdown.started",
        service = "payvia-platform"
    );
}
