use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use async_nats::ConnectOptions;
use tokio::{net::TcpListener, sync::watch};

use bacekend_pasarela::{
    reconciliation_worker::{self, ReconciliationWorkerConfig},
    webhooks::{
        GatewayWebhookState, load_account_registry, load_event_signing_key, router,
    },
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let address = env::var("GATEWAY_HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8081".to_owned())
        .parse::<SocketAddr>()?;
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
    let nats_user =
        env::var("GATEWAY_NATS_USER").map_err(|_| "GATEWAY_NATS_USER is required")?;
    let nats_password =
        env::var("GATEWAY_NATS_PASSWORD").map_err(|_| "GATEWAY_NATS_PASSWORD is required")?;
    if nats_user.trim().is_empty() || nats_password.len() < 32 {
        return Err("gateway NATS credentials are invalid".into());
    }

    let options = build_nats_options(nats_user, nats_password)
        .map_err(std::io::Error::other)?;
    let nats = options.connect(&nats_url).await?;
    let jetstream = async_nats::jetstream::new(nats);
    let state = GatewayWebhookState::new(accounts, jetstream.clone(), &signing_key, signing_key_id)?;
    let worker_config = ReconciliationWorkerConfig {
        platform_request_key: Arc::from(platform_request_key),
        platform_request_key_id: Arc::from(platform_request_key_id),
    };
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(reconciliation_worker::run(
        state.clone(),
        jetstream,
        worker_config,
        shutdown_rx,
    ));
    let listener = TcpListener::bind(address).await?;
    tracing::info!(
        event.name = "service.started",
        service = "payvia-gateway",
        http_addr = %listener.local_addr()?,
        provider = "wompi",
        callbacks = "enabled",
        reconciliation_worker = "enabled"
    );
    let server_result = axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    let _ = shutdown_tx.send(true);
    worker.await?;
    server_result?;
    Ok(())
}

fn build_nats_options(
    username: String,
    password: String,
) -> Result<ConnectOptions, &'static str> {
    let root = env::var_os("NATS_TLS_ROOT_CA");
    let certificate = env::var_os("NATS_TLS_CLIENT_CERT");
    let private_key = env::var_os("NATS_TLS_CLIENT_KEY");
    let required = env::var("PAYVIA_REQUIRE_MTLS")
        .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    match (root, certificate, private_key) {
        (Some(root), Some(certificate), Some(private_key)) => Ok(
            ConnectOptions::with_user_and_password(username, password)
                .add_root_certificates(PathBuf::from(root))
                .add_client_certificate(
                    PathBuf::from(certificate),
                    PathBuf::from(private_key),
                )
                .require_tls(true),
        ),
        (None, None, None) if !required => {
            Ok(ConnectOptions::with_user_and_password(username, password))
        }
        (None, None, None) => Err("gateway_nats_mtls_required"),
        _ => Err("gateway_nats_mtls_configuration_incomplete"),
    }
}
