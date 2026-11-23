use std::{env, net::SocketAddr, path::PathBuf, str::FromStr};
use thiserror::Error;

pub struct AppConfig {
    pub database_url: String,
    pub redis_url: String,
    pub redis_tls_root_ca: Option<PathBuf>,
    pub redis_tls_client_cert: Option<PathBuf>,
    pub redis_tls_client_key: Option<PathBuf>,
    pub nats_tls_root_ca: Option<PathBuf>,
    pub nats_tls_client_cert: Option<PathBuf>,
    pub nats_tls_client_key: Option<PathBuf>,
    pub require_mtls: bool,
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
        let environment = value("APP_ENV", "development");
        let require_mtls = boolean("PAYVIA_REQUIRE_MTLS", false)?;
        let database_url = database_url()?;
        let redis_url = redis_url()?;
        let nats_url = nats_url()?;
        let config = Self {
            database_url: if require_mtls {
                database_url
            } else {
                plaintext_database_url(&database_url)?
            },
            redis_url: if require_mtls {
                redis_url
            } else {
                plaintext_service_url(&redis_url, "rediss", "redis", "PLATFORM_REDIS_URL")?
            },
            redis_tls_root_ca: require_mtls
                .then(|| optional_path("PLATFORM_REDIS_TLS_ROOT_CA"))
                .flatten(),
            redis_tls_client_cert: require_mtls
                .then(|| optional_path("PLATFORM_REDIS_TLS_CLIENT_CERT"))
                .flatten(),
            redis_tls_client_key: require_mtls
                .then(|| optional_path("PLATFORM_REDIS_TLS_CLIENT_KEY"))
                .flatten(),
            nats_tls_root_ca: require_mtls
                .then(|| optional_path("NATS_TLS_ROOT_CA"))
                .flatten(),
            nats_tls_client_cert: require_mtls
                .then(|| optional_path("NATS_TLS_CLIENT_CERT"))
                .flatten(),
            nats_tls_client_key: require_mtls
                .then(|| optional_path("NATS_TLS_CLIENT_KEY"))
                .flatten(),
            require_mtls,
            nats_url: if require_mtls {
                nats_url
            } else {
                plaintext_service_url(&nats_url, "tls", "nats", "PLATFORM_NATS_URL")?
            },
            nats_username: nats_credential("NATS_USER", require_mtls)?,
            nats_password: nats_credential("NATS_PASSWORD", require_mtls)?,
            http_addr: address("APP_HTTP_ADDR", "0.0.0.0:8080")?,
            grpc_addr: address("APP_MODULE_GRPC_ADDR", "127.0.0.1:50051")?,
            metrics_addr: address("APP_METRICS_ADDR", "127.0.0.1:9090")?,
            environment,
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
    let has_username = !config.nats_username.trim().is_empty();
    let has_password = !config.nats_password.is_empty();
    if !validate_nats_url(&config.nats_url)
        || (has_username != has_password)
        || (!config.require_mtls && (!has_username || config.nats_password.len() < 32))
        || (config.require_mtls
            && has_username
            && (config.nats_username.trim().is_empty() || config.nats_password.len() < 32))
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
    let redis_tls_paths = [
        config.redis_tls_root_ca.is_some(),
        config.redis_tls_client_cert.is_some(),
        config.redis_tls_client_key.is_some(),
    ];
    let nats_tls_paths = [
        config.nats_tls_root_ca.is_some(),
        config.nats_tls_client_cert.is_some(),
        config.nats_tls_client_key.is_some(),
    ];
    if redis_tls_paths.iter().any(|configured| *configured)
        && !redis_tls_paths.iter().all(|configured| *configured)
    {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_TLS_CLIENT_CERT",
        });
    }
    if matches!(config.environment.as_str(), "production" | "staging") && !config.require_mtls {
        return Err(ConfigError::InvalidValue {
            variable: "PAYVIA_REQUIRE_MTLS",
        });
    }
    if config.require_mtls && !validate_production_database_url(&config.database_url) {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_DATABASE_URL",
        });
    }
    if config.require_mtls
        && url::Url::parse(&config.redis_url).is_ok_and(|url| url.scheme() != "rediss")
    {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_URL",
        });
    }
    if config.require_mtls && !redis_tls_paths.iter().all(|configured| *configured) {
        return Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_TLS_CLIENT_CERT",
        });
    }
    if config.require_mtls && !nats_tls_paths.iter().all(|configured| *configured) {
        return Err(ConfigError::InvalidValue {
            variable: "NATS_TLS_CLIENT_CERT",
        });
    }
    if config.require_mtls && !config.nats_url.starts_with("tls://") {
        return Err(ConfigError::InvalidValue {
            variable: "PAYVIA_REQUIRE_MTLS",
        });
    }
    if !config.require_mtls
        && (url::Url::parse(&config.database_url).is_ok_and(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "sslmode")
                .is_none_or(|(_, value)| value != "disable")
        }) || url::Url::parse(&config.redis_url).is_ok_and(|url| url.scheme() != "redis")
            || !config.nats_url.starts_with("nats://")
            || redis_tls_paths.iter().any(|configured| *configured)
            || nats_tls_paths.iter().any(|configured| *configured))
    {
        return Err(ConfigError::InvalidValue {
            variable: "PAYVIA_REQUIRE_MTLS",
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
                !matches!(key.as_ref(), "password" | "sslpassword")
                    && if matches!(key.as_ref(), "sslrootcert" | "sslcert" | "sslkey") {
                        value.len() <= 512
                    } else {
                        value.len() <= 32
                    }
            })
    })
}

fn validate_production_database_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        let parameters = url.query_pairs().collect::<Vec<_>>();
        ["sslmode", "sslrootcert", "sslcert", "sslkey"]
            .iter()
            .all(|required| {
                parameters
                    .iter()
                    .any(|(key, value)| key == required && !value.is_empty())
            })
            && parameters
                .iter()
                .any(|(key, value)| key == "sslmode" && value == "verify-full")
    })
}

pub fn plaintext_database_url(value: &str) -> Result<String, ConfigError> {
    let mut url = url::Url::parse(value).map_err(|_| ConfigError::InvalidValue {
        variable: "PLATFORM_DATABASE_URL",
    })?;
    let parameters = url
        .query_pairs()
        .filter(|(key, _)| {
            !matches!(
                key.as_ref(),
                "sslmode" | "sslrootcert" | "sslcert" | "sslkey"
            )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in parameters {
            query.append_pair(&key, &value);
        }
        query.append_pair("sslmode", "disable");
    }
    Ok(url.into())
}

fn plaintext_service_url(
    value: &str,
    secure_scheme: &str,
    plaintext_scheme: &str,
    variable: &'static str,
) -> Result<String, ConfigError> {
    let mut url = url::Url::parse(value).map_err(|_| ConfigError::InvalidValue { variable })?;
    if url.scheme() == secure_scheme {
        url.set_scheme(plaintext_scheme)
            .map_err(|_| ConfigError::InvalidValue { variable })?;
    }
    Ok(url.into())
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

fn nats_credential(variable: &'static str, require_mtls: bool) -> Result<String, ConfigError> {
    if require_mtls {
        Ok(optional(variable).unwrap_or_default())
    } else {
        required(variable)
    }
}

fn optional_path(variable: &'static str) -> Option<PathBuf> {
    env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn boolean(variable: &'static str, default: bool) -> Result<bool, ConfigError> {
    match env::var(variable) {
        Ok(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Ok(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        Ok(_) => Err(ConfigError::InvalidValue { variable }),
        Err(_) => Ok(default),
    }
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
