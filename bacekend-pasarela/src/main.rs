use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use async_nats::ConnectOptions;
use payvia_observability::telemetry::{TelemetryConfig, initialize};
use tokio::{net::TcpListener, sync::watch};

use bacekend_pasarela::{
    metrics,
    reconciliation_worker::{self, ReconciliationWorkerConfig},
    webhooks::{GatewayWebhookState, load_account_registry, load_event_signing_key, router},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let environment = env::var("GATEWAY_ENV").unwrap_or_else(|_| "development".to_owned());
    let release =
        env::var("GATEWAY_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned());
    let otlp_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    let sentry_dsn = env::var("GATEWAY_SENTRY_DSN").ok();
    let sentry_traces_sample_rate = env::var("SENTRY_TRACES_SAMPLE_RATE")
        .ok()
        .map(|value| value.parse::<f32>())
        .transpose()?;
    if sentry_traces_sample_rate
        .is_some_and(|rate| !rate.is_finite() || !(0.0..=1.0).contains(&rate))
    {
        return Err("SENTRY_TRACES_SAMPLE_RATE must be between 0 and 1".into());
    }
    let telemetry_runtime = initialize(&TelemetryConfig {
        service_name: "payvia-gateway",
        environment: &environment,
        release: &release,
        otlp_endpoint: otlp_endpoint.as_deref(),
        sentry_dsn: sentry_dsn.as_deref(),
        sentry_traces_sample_rate,
    })?;

    let address = env::var("GATEWAY_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8081".to_owned())
        .parse::<SocketAddr>()?;
    let metrics_address = env::var("GATEWAY_METRICS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9091".to_owned())
        .parse::<SocketAddr>()?;
    let metrics_token = env::var("PASARELA_METRICS_TOKEN").ok();
    if !metrics_address.ip().is_loopback()
        && metrics_token
            .as_deref()
            .is_none_or(|token| token.len() < 32)
    {
        return Err(
            "PASARELA_METRICS_TOKEN must contain at least 32 bytes when metrics are exposed".into(),
        );
    }
    let account_registry = env::var("GATEWAY_WOMPI_ACCOUNTS_JSON")
        .map_err(|_| "GATEWAY_WOMPI_ACCOUNTS_JSON is required")?;
    let accounts = load_account_registry(&account_registry)?;
    let signing_key = env::var("GATEWAY_EVENT_SIGNING_KEY")
        .map_err(|_| "GATEWAY_EVENT_SIGNING_KEY is required")?;
    let signing_key = load_event_signing_key(&signing_key)?;
    let signing_key_id = env::var("GATEWAY_EVENT_SIGNING_KEY_ID")
        .map_err(|_| "GATEWAY_EVENT_SIGNING_KEY_ID is required")?;
    let platform_request_key = env::var("GATEWAY_PLATFORM_REQUEST_VERIFY_KEY")
        .map_err(|_| "GATEWAY_PLATFORM_REQUEST_VERIFY_KEY is required")?;
    let platform_request_key = load_event_signing_key(&platform_request_key)?;
    let platform_request_key_id = env::var("GATEWAY_PLATFORM_REQUEST_VERIFY_KEY_ID")
        .map_err(|_| "GATEWAY_PLATFORM_REQUEST_VERIFY_KEY_ID is required")?;
    let nats_url =
        env::var("GATEWAY_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned());
    if environment == "production" && !nats_url.starts_with("tls://") {
        return Err("gateway_nats_tls_required".into());
    }
    let mapped_identity = environment == "production";
    let nats_user = env::var("GATEWAY_NATS_USER").unwrap_or_default();
    let nats_password = env::var("GATEWAY_NATS_PASSWORD").unwrap_or_default();
    if !mapped_identity && (nats_user.trim().is_empty() || nats_password.len() < 32) {
        return Err("gateway NATS credentials are invalid".into());
    }

    let options = build_nats_options(nats_user, nats_password).map_err(|error_code| {
        tracing::error!(
            event.name = "service.configuration.invalid",
            component = "gateway_runtime",
            operation = "configure_nats_mtls",
            error_code,
            error_category = "configuration",
            retryable = false
        );
        std::io::Error::other(error_code)
    })?;
    let nats = options.connect(&nats_url).await.inspect_err(|error| {
        tracing::error!(
            event.name = "service.startup.dependency.failed",
            component = "gateway_runtime",
            operation = "connect_nats",
            dependency = "nats_jetstream",
            error_code = "nats_connect_failed",
            error_category = "connectivity",
            retryable = true,
            error_class = %error
        );
    })?;
    let jetstream = async_nats::jetstream::new(nats);
    let state =
        GatewayWebhookState::new(accounts, jetstream.clone(), &signing_key, signing_key_id)?;
    let worker_config = ReconciliationWorkerConfig {
        platform_request_key: Arc::from(platform_request_key),
        platform_request_key_id: Arc::from(platform_request_key_id),
    };
    let metrics_handle = metrics::install_recorder()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(reconciliation_worker::run(
        state.clone(),
        jetstream,
        worker_config,
        shutdown_rx.clone(),
    ));
    let metrics_listener = TcpListener::bind(metrics_address).await?;
    let metrics_task = tokio::spawn(metrics::serve(
        metrics_listener,
        metrics_handle,
        metrics_token,
        shutdown_rx.clone(),
    ));
    let listener = TcpListener::bind(address).await?;
    tracing::info!(
        event.name = "service.started",
        service = "payvia-gateway",
        http_addr = %listener.local_addr()?,
        provider = "wompi",
        callbacks = "enabled",
        reconciliation_worker = "enabled",
        metrics_addr = %metrics_address,
        environment = %environment,
        release = %release
    );
    let server_result = axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    let _ = shutdown_tx.send(true);
    worker.await?;
    metrics_task.await??;
    server_result?;
    telemetry_runtime.tracer_provider.shutdown()?;
    telemetry_runtime.logger_provider.shutdown()?;
    Ok(())
}

fn build_nats_options(username: String, password: String) -> Result<ConnectOptions, &'static str> {
    let root = env::var_os("NATS_TLS_ROOT_CA").filter(|value| !value.is_empty());
    let certificate = env::var_os("NATS_TLS_CLIENT_CERT").filter(|value| !value.is_empty());
    let private_key = env::var_os("NATS_TLS_CLIENT_KEY").filter(|value| !value.is_empty());
    let mapped_identity =
        env::var("GATEWAY_ENV").is_ok_and(|value| value.eq_ignore_ascii_case("production"));
    let required = mapped_identity
        || env::var("PAYVIA_REQUIRE_MTLS").is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    match (root, certificate, private_key) {
        (Some(root), Some(certificate), Some(private_key)) if mapped_identity => {
            Ok(ConnectOptions::new()
                .add_root_certificates(PathBuf::from(root))
                .add_client_certificate(PathBuf::from(certificate), PathBuf::from(private_key))
                .require_tls(true))
        }
        (Some(root), Some(certificate), Some(private_key)) => {
            Ok(ConnectOptions::with_user_and_password(username, password)
                .add_root_certificates(PathBuf::from(root))
                .add_client_certificate(PathBuf::from(certificate), PathBuf::from(private_key))
                .require_tls(true))
        }
        (None, None, None) if !required => {
            Ok(ConnectOptions::with_user_and_password(username, password))
        }
        (None, None, None) => Err("gateway_nats_mtls_required"),
        _ => Err("gateway_nats_mtls_configuration_incomplete"),
    }
}
