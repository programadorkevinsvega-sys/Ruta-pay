use async_nats::jetstream;
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use std::time::Duration;
use tokio::{sync::watch, time};

pub(super) async fn run_realtime_consumer(
    pool: PgPool,
    redis_url: String,
    consumer: jetstream::consumer::PullConsumer,
    mut shutdown: watch::Receiver<bool>,
) {
    let redis_client = match redis::Client::open(redis_url) {
        Ok(client) => client,
        Err(_) => {
            tracing::error!(
                event.name = "realtime.redis.client.invalid",
                component = "realtime",
                operation = "create_redis_client",
                dependency = "redis",
                error_code = "redis_url_invalid",
                error_category = "configuration",
                retryable = false
            );
            return;
        }
    };
    let mut backoff = Duration::from_millis(100);
    let mut redis = loop {
        match redis::aio::ConnectionManager::new(redis_client.clone()).await {
            Ok(redis) => break redis,
            Err(error) => {
                tracing::warn!(event.name = "realtime.redis.connection.failed", component = "realtime", operation = "connect_signal_publisher", dependency = "redis", error_code = "redis_connect_failed", error_category = "connectivity", retryable = true, error_class = %error);
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        }
    };
    loop {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(event.name = "realtime.jetstream.fetch.failed", component = "realtime", operation = "fetch_journal_events", dependency = "nats_jetstream", error_code = "realtime_consumer_fetch_failed", error_category = "connectivity", retryable = true, error_class = %error);
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        loop {
            tokio::select! {
                result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                message = messages.next() => match message {
                    Some(Ok(message)) => match platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(&pool, &mut redis, &message.payload).await {
                        Ok(inserted) => {
                            if let Err(error) = message.ack().await {
                                tracing::error!(event.name = "realtime.jetstream.ack.failed", component = "realtime", operation = "ack_journal_event", dependency = "nats_jetstream", error_code = "realtime_ack_failed", error_category = "connectivity", retryable = true, error_class = %error);
                            } else if inserted {
                                tracing::info!(event.name = "realtime.journal.event.persisted", component = "realtime", operation = "persist_journal_event", result = "success");
                            }
                        }
                        Err(error) => {
                            tracing::error!(event.name = "realtime.journal.event.failed", component = "realtime", operation = "persist_and_signal", dependency = "postgresql_or_redis", error_code = "realtime_event_processing_failed", error_category = "dependency", retryable = true, error_class = %error);
                            if let Err(ack_error) = message.ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5)))).await {
                                tracing::error!(event.name = "realtime.jetstream.nak.failed", component = "realtime", operation = "nak_journal_event", dependency = "nats_jetstream", error_code = "realtime_nak_failed", error_category = "connectivity", retryable = true, error_class = %ack_error);
                            }
                            if matches!(error, platform_realtime::infrastructure::persistence::JournalError::Redis) {
                                match redis::aio::ConnectionManager::new(redis_client.clone()).await {
                                    Ok(connection) => redis = connection,
                                    Err(connection_error) => tracing::warn!(event.name = "realtime.redis.reconnect.failed", component = "realtime", operation = "reconnect_signal_publisher", dependency = "redis", error_code = "redis_reconnect_failed", error_category = "connectivity", retryable = true, error_class = %connection_error),
                                }
                            }
                        }
                    },
                    Some(Err(error)) => {
                        tracing::warn!(event.name = "realtime.jetstream.stream.failed", component = "realtime", operation = "read_journal_events", dependency = "nats_jetstream", error_code = "realtime_consumer_stream_failed", error_category = "connectivity", retryable = true, error_class = %error);
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}
