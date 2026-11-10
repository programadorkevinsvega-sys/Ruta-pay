use std::{env, net::SocketAddr, str::FromStr};
use thiserror::Error;

pub struct AppConfig {
    pub database_url: String,
    pub redis_url: String,
    pub nats_url: String,
    pub nats_username: String,
    pub nats_password: String,
    pub http_addr: SocketAddr,
    pub grpc_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
    pub environment: String,
    pub version: String,
    pub otlp_endpoint: Option<String>,
    pub sentry_dsn: Option<String>,
    pub sentry_traces_sample_rate: Option<f32>,
    pub metrics_token: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid socket address for {variable}")]
    InvalidAddress { variable: &'static str },
    #[error("invalid value for {variable}")]
    InvalidValue { variable: &'static str },
}

impl ConfigError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidAddress { .. } => "invalid_socket_address",
            Self::InvalidValue { .. } => "invalid_configuration_value",
        }
    }

    pub fn variable(&self) -> &'static str {
        match self {
            Self::InvalidAddress { variable } | Self::InvalidValue { variable } => variable,
        }
    }
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let config = Self {
            database_url: database_url()?,
            redis_url: redis_url()?,
            nats_url: nats_url()?,
            nats_username: required("NATS_USER")?,
            nats_password: required("NATS_PASSWORD")?,
            http_addr: address("APP_HTTP_ADDR", "0.0.0.0:8080")?,
            grpc_addr: address("APP_MODULE_GRPC_ADDR", "127.0.0.1:50051")?,
            metrics_addr: address("APP_METRICS_ADDR", "127.0.0.1:9090")?,
            environment: value("APP_ENV", "development"),
            version: value("APP_VERSION", env!("CARGO_PKG_VERSION")),
            otlp_endpoint: otlp_endpoint()?,
            sentry_dsn: sentry_dsn()?,
            sentry_traces_sample_rate: sentry_traces_sample_rate()?,
            metrics_token: optional("APP_METRICS_BEARER_TOKEN"),
        };
        validate_config(&config)?;
        Ok(config)
    }
}

fn validate_config(config: &AppConfig) -> Result<(), ConfigError> {
    if config
        .sentry_traces_sample_rate
        .is_some_and(|rate| !valid_sentry_traces_sample_rate(rate))
    {
        return Err(ConfigError::InvalidValue {
            variable: "SENTRY_TRACES_SAMPLE_RATE",
        });
    }
    if !validate_database_url(&config.database_url) {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_DATABASE_URL",
        });
    }
    if !validate_redis_url(&config.redis_url) {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_URL",
        });
    }
    if !validate_nats_url(&config.nats_url)
        || config.nats_username.trim().is_empty()
        || config.nats_password.len() < 32
    {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_NATS_URL",
        });
    }
    if !["development", "test", "staging", "production"].contains(&config.environment.as_str()) {
        return Err(ConfigError::InvalidValue {
            variable: "APP_ENV",
        });
    }
    if !config.grpc_addr.ip().is_loopback() {
        return Err(ConfigError::InvalidValue {
            variable: "APP_MODULE_GRPC_ADDR",
        });
    }
    if !config.metrics_addr.ip().is_loopback()
        && config
            .metrics_token
            .as_deref()
            .is_none_or(|token| token.len() < 32)
    {
        return Err(ConfigError::InvalidValue {
            variable: "APP_METRICS_BEARER_TOKEN",
        });
    }
    Ok(())
}

fn database_url() -> Result<String, ConfigError> {
    let value = env::var("PLATFORM_DATABASE_URL").map_err(|_| ConfigError::InvalidValue {
        variable: "PLATFORM_DATABASE_URL",
    })?;
    if !validate_database_url(&value) {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_DATABASE_URL",
        });
    }
    Ok(value)
}

fn validate_database_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "postgres"
            && url.host_str().is_some()
            && !url.username().is_empty()
            && url.password().is_some()
            && url.query_pairs().all(|(key, value)| {
                !matches!(key.as_ref(), "password" | "sslpassword") && value.len() <= 32
            })
    })
}

fn redis_url() -> Result<String, ConfigError> {
    let value = env::var("PLATFORM_REDIS_URL").map_err(|_| ConfigError::InvalidValue {
        variable: "PLATFORM_REDIS_URL",
    })?;
    if !validate_redis_url(&value) {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_URL",
        });
    }
    Ok(value)
}

fn validate_redis_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        ["redis", "rediss"].contains(&url.scheme())
            && url.host_str().is_some()
            && url.password().is_some()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn nats_url() -> Result<String, ConfigError> {
    let value =
        env::var("PLATFORM_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned());
    if !validate_nats_url(&value) {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_NATS_URL",
        });
    }
    Ok(value)
}

fn validate_nats_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        ["nats", "tls"].contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn required(variable: &'static str) -> Result<String, ConfigError> {
    env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigError::InvalidValue { variable })
}

fn address(variable: &'static str, default: &str) -> Result<SocketAddr, ConfigError> {
    let value = env::var(variable).unwrap_or_else(|_| default.to_owned());
    parse_address(variable, &value)
}

fn parse_address(variable: &'static str, value: &str) -> Result<SocketAddr, ConfigError> {
    SocketAddr::from_str(value).map_err(|_| ConfigError::InvalidAddress { variable })
}

fn value(variable: &str, default: &str) -> String {
    env::var(variable).unwrap_or_else(|_| default.to_owned())
}

fn otlp_endpoint() -> Result<Option<String>, ConfigError> {
    let endpoint = optional("OTEL_EXPORTER_OTLP_ENDPOINT");
    if endpoint
        .as_deref()
        .is_some_and(|value| !validate_otlp_endpoint(value))
    {
        return Err(ConfigError::InvalidValue {
            variable: "OTEL_EXPORTER_OTLP_ENDPOINT",
        });
    }
    Ok(endpoint)
}

fn validate_otlp_endpoint(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        ["http", "https"].contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn sentry_dsn() -> Result<Option<String>, ConfigError> {
    let dsn = optional("SENTRY_DSN");
    if dsn
        .as_deref()
        .is_some_and(|value| value.parse::<sentry::types::Dsn>().is_err())
    {
        return Err(ConfigError::InvalidValue {
            variable: "SENTRY_DSN",
        });
    }
    Ok(dsn)
}

fn sentry_traces_sample_rate() -> Result<Option<f32>, ConfigError> {
    let rate = optional("SENTRY_TRACES_SAMPLE_RATE")
        .map(|value| {
            value.parse::<f32>().map_err(|_| ConfigError::InvalidValue {
                variable: "SENTRY_TRACES_SAMPLE_RATE",
            })
        })
        .transpose()?;
    if rate.is_some_and(|value| !valid_sentry_traces_sample_rate(value)) {
        return Err(ConfigError::InvalidValue {
            variable: "SENTRY_TRACES_SAMPLE_RATE",
        });
    }
    Ok(rate)
}

fn valid_sentry_traces_sample_rate(rate: f32) -> bool {
    rate.is_finite() && (0.0..=1.0).contains(&rate)
}

fn optional(variable: &str) -> Option<String> {
    env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
#[path = "app_config/tests.rs"]
mod tests;
