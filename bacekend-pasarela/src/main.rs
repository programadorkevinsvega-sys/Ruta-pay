use std::{env, net::SocketAddr};

use async_nats::ConnectOptions;
use tokio::net::TcpListener;

use bacekend_pasarela::webhooks::{
    GatewayWebhookState, load_account_registry, load_event_signing_key, router,
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
    let nats_url =
        env::var("GATEWAY_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned());
    let nats_user = env::var("GATEWAY_NATS_USER")
        .map_err(|_| "GATEWAY_NATS_USER is required")?;
    let nats_password = env::var("GATEWAY_NATS_PASSWORD")
        .map_err(|_| "GATEWAY_NATS_PASSWORD is required")?;
    if nats_user.trim().is_empty() || nats_password.len() < 32 {
        return Err("gateway NATS credentials are invalid".into());
    }

    let nats = ConnectOptions::with_user_and_password(nats_user, nats_password)
        .connect(&nats_url)
        .await?;
    let jetstream = async_nats::jetstream::new(nats);
    let state = GatewayWebhookState::new(accounts, jetstream, &signing_key, signing_key_id)?;
    let listener = TcpListener::bind(address).await?;
    tracing::info!(
        event.name = "service.started",
        service = "payvia-gateway",
        http_addr = %listener.local_addr()?,
        provider = "wompi",
        callbacks = "enabled"
    );
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
