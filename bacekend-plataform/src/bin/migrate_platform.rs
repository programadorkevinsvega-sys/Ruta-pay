#[path = "migrate_platform/migration.rs"]
mod migration;
#[path = "migrate_platform/privileges.rs"]
mod privileges;
#[path = "migrate_platform/validation.rs"]
mod validation;

use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    match migration::migrate().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => {
            tracing::error!(
                event.name = "database.migration.failed",
                component = "platform_migrations",
                operation = "apply_coordinated_migrations",
                dependency = "postgresql",
                error_code = code,
                error_category = "persistence",
                retryable = false
            );
            ExitCode::FAILURE
        }
    }
}
