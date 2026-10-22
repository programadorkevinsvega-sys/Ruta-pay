use crate::config::AppConfig;
use payvia_observability::telemetry::TelemetryConfig;
pub use payvia_observability::telemetry::{TelemetryInitError, TelemetryRuntime};

pub fn initialize(config: &AppConfig) -> Result<TelemetryRuntime, TelemetryInitError> {
    payvia_observability::telemetry::initialize(&TelemetryConfig {
        service_name: "payvia-platform",
        environment: &config.environment,
        release: &config.version,
        otlp_endpoint: config.otlp_endpoint.as_deref(),
        sentry_dsn: config.sentry_dsn.as_deref(),
    })
}
