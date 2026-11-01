mod dependencies;
mod support;

use support::{bind_listener, serve_runtime, verify_runtime_startup};
use payvia_platform_bff::http::routes::router;
use payvia_platform_core::{config::AppConfig, metrics, shutdown_signal, telemetry};
use platform_contracts::catalog_pricing::catalog_pricing_client::CatalogPricingClient as CatalogPricingGrpcClient;
use platform_contracts::entitlements_quotas::entitlements_quotas_client::EntitlementsQuotasClient as EntitlementsQuotasGrpcClient;
use platform_contracts::identity_access::identity_access_client::IdentityAccessClient as IdentityAccessGrpcClient;
use platform_contracts::realtime::realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient;
use platform_contracts::runtime::runtime_probe_client::RuntimeProbeClient as RuntimeProbeGrpcClient;
use platform_contracts::tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient;
use std::time::Duration;
use tokio::sync::watch;
use tonic::transport::Endpoint;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
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
    let database_pool = dependencies::connect_database(&config).await?;
    let (redis_client, redis_manager) = dependencies::connect_redis(&config).await?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let event_task = tokio::spawn(platform_runtime::event_workers::run(
        database_pool.clone(),
        config.redis_url.clone(),
        config.nats_url.clone(),
        config.nats_username.clone(),
        config.nats_password.clone(),
        shutdown_rx.clone(),
    ));
    let runtime_listener = bind_listener(config.grpc_addr, "platform_runtime", "bind_grpc").await?;
    let runtime_addr = runtime_listener.local_addr()?;
    let runtime_channel = Endpoint::from_shared(format!("http://{runtime_addr}"))?.connect_lazy();
    let identity_module_client = IdentityAccessGrpcClient::new(runtime_channel.clone());
    let catalog_module_client = CatalogPricingGrpcClient::new(runtime_channel.clone());
    let tenant_module_client = TenantRegistryGrpcClient::new(runtime_channel);
    let runtime_task = tokio::spawn(serve_runtime(
        runtime_listener,
        platform_runtime::module_config::RuntimeModuleDependencies {
            pool: database_pool,
            redis: redis_manager,
            redis_client,
            identity_client: identity_module_client,
            catalog_client: catalog_module_client,
            tenant_client: tenant_module_client,
        },
        shutdown_rx.clone(),
    ));
    let endpoint = Endpoint::from_shared(format!("http://{runtime_addr}"))?
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2));
    let grpc_client = RuntimeProbeGrpcClient::connect(endpoint).await.map_err(|status| {
        tracing::error!(event.name = "service.startup.dependency.failed", component = "platform_bff", operation = "connect_runtime", dependency = "platform_runtime", error_code = "grpc_connect_failed", error_category = "connectivity", dependency_error = %status, retryable = true);
        status
    })?;
    verify_runtime_startup(grpc_client.clone()).await.inspect_err(|status| {
        tracing::error!(event.name = "service.startup.readiness.failed", component = "platform_bff", operation = "startup_readiness", dependency = "platform_runtime", error_code = %status.code(), error_category = "dependency", retryable = status.code() == tonic::Code::Unavailable || status.code() == tonic::Code::DeadlineExceeded);
    })?;
    let identity_endpoint = Endpoint::from_shared(format!("http://{runtime_addr}"))?
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5));
    let identity_client = IdentityAccessGrpcClient::connect(identity_endpoint)
        .await
        .inspect_err(|_| {
            tracing::error!(
                event.name = "service.startup.dependency.failed",
                component = "platform_bff",
                operation = "connect_identity_access",
                dependency = "identity_access",
                error_code = "grpc_connect_failed",
                error_category = "connectivity",
                retryable = true
            );
        })?;
    let tenant_endpoint = Endpoint::from_shared(format!("http://{runtime_addr}"))?
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5));
    let tenant_client = TenantRegistryGrpcClient::connect(tenant_endpoint)
        .await
        .inspect_err(|_| {
            tracing::error!(
                event.name = "service.startup.dependency.failed",
                component = "platform_bff",
                operation = "connect_tenant_registry",
                dependency = "tenant_registry",
                error_code = "grpc_connect_failed",
                error_category = "connectivity",
                retryable = true
            )
        })?;
    let realtime_endpoint = Endpoint::from_shared(format!("http://{runtime_addr}"))?
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5));
    let realtime_client = RealtimeGatewayGrpcClient::connect(realtime_endpoint)
        .await
        .inspect_err(|_| {
            tracing::error!(
                event.name = "service.startup.dependency.failed",
                component = "platform_bff",
                operation = "connect_realtime_gateway",
                dependency = "realtime",
                error_code = "grpc_connect_failed",
                error_category = "connectivity",
                retryable = true
            )
        })?;
    let catalog_client = CatalogPricingGrpcClient::connect(
        Endpoint::from_shared(format!("http://{runtime_addr}"))?
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5)),
    )
    .await?;
    let entitlements_client = EntitlementsQuotasGrpcClient::connect(
        Endpoint::from_shared(format!("http://{runtime_addr}"))?
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5)),
    )
    .await?;
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
    axum::serve(
        http_listener,
        router(
            grpc_client,
            identity_client,
            tenant_client,
            realtime_client,
            catalog_client,
            entitlements_client,
        )
        .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal::wait_for_shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    })
    .await?;
    runtime_task.await??;
    metrics_task.await??;
    event_task.await?;
    telemetry.tracer_provider.shutdown()?;
    telemetry.logger_provider.shutdown()?;
    drop(telemetry.sentry_guard);
    Ok(())
}
