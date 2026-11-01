use payvia_platform_core::config::AppConfig;
use std::time::Duration;

pub async fn connect_database(
    config: &AppConfig,
) -> Result<sqlx::PgPool, sqlx::Error> {
    let database_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
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

pub async fn connect_redis(
    config: &AppConfig,
) -> Result<(redis::Client, redis::aio::ConnectionManager), redis::RedisError> {
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
