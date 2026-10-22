use std::{env, net::SocketAddr, str::FromStr};
use thiserror::Error;

pub struct AppConfig {
    pub http_addr: SocketAddr,
    pub grpc_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
    pub environment: String,
    pub version: String,
    pub otlp_endpoint: Option<String>,
    pub sentry_dsn: Option<String>,
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
            http_addr: address("APP_HTTP_ADDR", "0.0.0.0:8080")?,
            grpc_addr: address("APP_MODULE_GRPC_ADDR", "127.0.0.1:50051")?,
            metrics_addr: address("APP_METRICS_ADDR", "127.0.0.1:9090")?,
            environment: value("APP_ENV", "development"),
            version: value("APP_VERSION", env!("CARGO_PKG_VERSION")),
            otlp_endpoint: otlp_endpoint()?,
            sentry_dsn: sentry_dsn()?,
            metrics_token: optional("APP_METRICS_BEARER_TOKEN"),
        };
        validate_config(&config)?;
        Ok(config)
    }
}

fn validate_config(config: &AppConfig) -> Result<(), ConfigError> {
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

fn optional(variable: &str) -> Option<String> {
    env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_bind_metrics_locally() {
        let config = AppConfig::from_env().expect("defaults must be valid");
        assert_eq!(config.metrics_addr.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn otlp_endpoint_rejects_credentials_and_query_strings() {
        assert!(!validate_otlp_endpoint(
            "https://user:password@collector.example:4317"
        ));
        assert!(!validate_otlp_endpoint(
            "https://collector.example:4317?token=sensitive"
        ));
        assert!(validate_otlp_endpoint("https://collector.example:4317"));
    }

    #[test]
    fn non_loopback_grpc_listener_is_rejected_without_mtls() {
        let config = AppConfig {
            http_addr: "0.0.0.0:8080".parse().expect("address"),
            grpc_addr: "0.0.0.0:50051".parse().expect("address"),
            metrics_addr: "127.0.0.1:9090".parse().expect("address"),
            environment: "test".to_owned(),
            version: "test-version".to_owned(),
            otlp_endpoint: None,
            sentry_dsn: None,
            metrics_token: None,
        };
        assert_eq!(
            validate_config(&config),
            Err(ConfigError::InvalidValue {
                variable: "APP_MODULE_GRPC_ADDR"
            })
        );
    }

    #[test]
    fn non_loopback_metrics_requires_a_long_bearer_token() {
        let config = AppConfig {
            http_addr: "0.0.0.0:8080".parse().expect("address"),
            grpc_addr: "127.0.0.1:50051".parse().expect("address"),
            metrics_addr: "0.0.0.0:9090".parse().expect("address"),
            environment: "test".to_owned(),
            version: "test-version".to_owned(),
            otlp_endpoint: None,
            sentry_dsn: None,
            metrics_token: Some("short".to_owned()),
        };
        assert_eq!(
            validate_config(&config),
            Err(ConfigError::InvalidValue {
                variable: "APP_METRICS_BEARER_TOKEN"
            })
        );
    }

    #[test]
    fn invalid_bind_address_has_stable_error_code() {
        assert_eq!(
            parse_address("APP_HTTP_ADDR", "not-an-address"),
            Err(ConfigError::InvalidAddress {
                variable: "APP_HTTP_ADDR"
            })
        );
    }
}
