use std::{env, process::ExitCode};

use async_nats::{ConnectOptions, jetstream};
use platform_runtime::event_topology::delete_e2e_stream;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    match cleanup().await {
        Ok(removed) => {
            tracing::info!(
                event.name = "e2e.jetstream.cleanup.completed",
                component = "platform_event_runtime",
                operation = "delete_e2e_stream",
                result = if removed { "deleted" } else { "already_absent" }
            );
            ExitCode::SUCCESS
        }
        Err(error_code) => {
            tracing::error!(
                event.name = "e2e.jetstream.cleanup.failed",
                component = "platform_event_runtime",
                operation = "delete_e2e_stream",
                error_code,
                error_category = "dependency",
                retryable = false
            );
            ExitCode::FAILURE
        }
    }
}

async fn cleanup() -> Result<bool, &'static str> {
    let namespace = env::var("PLATFORM_EVENT_NAMESPACE").map_err(|_| "event_namespace_missing")?;
    let url = env::var("PLATFORM_NATS_URL").map_err(|_| "nats_url_missing")?;
    let username = env::var("NATS_USER").map_err(|_| "nats_user_missing")?;
    let password = env::var("NATS_PASSWORD").map_err(|_| "nats_password_missing")?;
    let client = ConnectOptions::with_user_and_password(username, password)
        .connect(url)
        .await
        .map_err(|_| "nats_connection_failed")?;
    delete_e2e_stream(&jetstream::new(client), &namespace).await
}
