use payvia_platform_core::config::AppConfig;
use std::{env, time::Duration};

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
    let redis_client = redis::Client::open(config.redis_url.as_str())
        .map_err(|_| std::io::Error::other("platform Redis URL is invalid"))?;
    let redis_config = redis::aio::ConnectionManagerConfig::new()
        .set_min_delay(Duration::from_millis(100))
        .set_max_delay(Duration::from_secs(2))
        .set_number_of_retries(10)
        .set_connection_timeout(Some(Duration::from_secs(2)))
        .set_response_timeout(Some(Duration::from_secs(2)));
    let redis_manager =
        redis::aio::ConnectionManager::new_with_config(redis_client.clone(), redis_config)
            .await
            .inspect_err(|_| {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_runtime",
                    operation = "connect_redis",
                    dependency = "redis",
                    error_code = "redis_connect_failed",
                    error_category = "connectivity",
                    retryable = true
                );
            })?;
    Ok((redis_client, redis_manager))
}
