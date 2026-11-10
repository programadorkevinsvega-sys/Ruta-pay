use payvia_platform_core::config::AppConfig;
use std::{env, fs, time::Duration};

pub async fn connect_database(config: &AppConfig) -> Result<sqlx::PgPool, sqlx::Error> {
    let (min_connections, max_connections) = configured_pool_limits();
    tracing::info!(
        event.name = "service.dependency.pool.configured",
        component = "platform_runtime",
        dependency = "postgresql",
        min_connections,
        max_connections
    );
    let database_pool = sqlx::postgres::PgPoolOptions::new()
        .min_connections(min_connections)
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&config.database_url)
        .await
        .inspect_err(|error| {
            tracing::error!(
                event.name = "service.startup.dependency.failed",
                component = "platform_runtime",
                operation = "connect_database",
                dependency = "postgresql",
                error_code = error
                    .as_database_error()
                    .and_then(|database| database.code())
                    .as_deref()
                    .unwrap_or("connection_failed"),
                error_category = "connectivity",
                retryable = true
            );
        })?;
    Ok(database_pool)
}

fn configured_pool_limits() -> (u32, u32) {
    let max = configured_connection_limit("PLATFORM_DATABASE_MAX_CONNECTIONS", 8, 2, 64);
    let min = configured_connection_limit("PLATFORM_DATABASE_MIN_CONNECTIONS", 2, 0, max);
    (min, max)
}

fn configured_connection_limit(name: &str, default: u32, minimum: u32, maximum: u32) -> u32 {
    match env::var(name) {
        Ok(value) => match value.parse::<u32>() {
            Ok(value) if (minimum..=maximum).contains(&value) => value,
            _ => {
                tracing::warn!(
                    event.name = "service.configuration.invalid",
                    component = "platform_runtime",
                    operation = "configure_database_pool",
                    error_code = "database_pool_connection_limit_invalid",
                    setting = name,
                    fallback = default.min(maximum).max(minimum)
                );
                default.min(maximum).max(minimum)
            }
        },
        Err(_) => default.min(maximum).max(minimum),
    }
}

pub async fn connect_redis(
    config: &AppConfig,
) -> Result<(redis::Client, redis::aio::ConnectionManager), Box<dyn std::error::Error>> {
    let redis_client = match (
        &config.redis_tls_root_ca,
        &config.redis_tls_client_cert,
        &config.redis_tls_client_key,
    ) {
        (Some(root_ca), Some(client_cert), Some(client_key)) => {
            let tls_certificates = redis::TlsCertificates {
                client_tls: Some(redis::ClientTlsConfig {
                    client_cert: read_redis_tls_file(
                        client_cert,
                        "PLATFORM_REDIS_TLS_CLIENT_CERT",
                    )?,
                    client_key: read_redis_tls_file(client_key, "PLATFORM_REDIS_TLS_CLIENT_KEY")?,
                }),
                root_cert: Some(read_redis_tls_file(root_ca, "PLATFORM_REDIS_TLS_ROOT_CA")?),
            };
            redis::Client::build_with_tls(config.redis_url.as_str(), tls_certificates).inspect_err(
                |error| {
                    tracing::error!(
                        event.name = "service.configuration.invalid",
                        component = "platform_runtime",
                        operation = "configure_redis_mtls",
                        dependency = "redis",
                        error_code = "redis_tls_identity_invalid",
                        error_category = "configuration",
                        error_class = ?error.kind(),
                        retryable = false
                    );
                },
            )?
        }
        (None, None, None) if config.environment != "production" && !config.require_mtls => {
            redis::Client::open(config.redis_url.as_str())?
        }
        _ => {
            return Err(
                std::io::Error::other("platform Redis mTLS configuration is incomplete").into(),
            );
        }
    };
    let redis_config = redis::aio::ConnectionManagerConfig::new()
        .set_min_delay(Duration::from_millis(100))
        .set_max_delay(Duration::from_secs(2))
        .set_number_of_retries(10)
        .set_connection_timeout(Some(Duration::from_secs(2)))
        .set_response_timeout(Some(Duration::from_secs(2)));
    let redis_manager =
        redis::aio::ConnectionManager::new_with_config(redis_client.clone(), redis_config)
            .await
            .inspect_err(|error| {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_runtime",
                    operation = "connect_redis",
                    dependency = "redis",
                    error_code = "redis_connect_failed",
                    error_category = "connectivity",
                    retryable = true,
                    error_class = ?error.kind()
                );
            })?;
    Ok((redis_client, redis_manager))
}

fn read_redis_tls_file(
    path: &std::path::Path,
    variable: &'static str,
) -> Result<Vec<u8>, std::io::Error> {
    fs::read(path).inspect_err(|error| {
        tracing::error!(
            event.name = "service.configuration.invalid",
            component = "platform_runtime",
            operation = "load_redis_mtls_identity",
            dependency = "redis",
            error_code = "redis_tls_file_unavailable",
            error_category = "configuration",
            variable,
            error_class = %error.kind(),
            retryable = false
        );
    })
}
