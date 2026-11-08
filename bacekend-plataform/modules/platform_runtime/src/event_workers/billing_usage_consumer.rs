use async_nats::jetstream;
use metrics::counter;
use sqlx::PgPool;
use std::{
    env,
    time::Duration,
};
use tokio::{sync::watch, task::JoinSet, time};
use tokio_stream::StreamExt;

mod processing;
use processing::process_message;

const DEFAULT_MAX_IN_FLIGHT: usize = 16;
const MAX_IN_FLIGHT_LIMIT: usize = 64;

pub(super) async fn run_billing_usage_consumer(
    pool: PgPool,
    consumer: jetstream::consumer::PullConsumer,
    mut shutdown: watch::Receiver<bool>,
) {
    let service = BillingService::new(pool.clone());
    let max_in_flight = configured_max_in_flight();
    tracing::info!(
        event.name = "jetstream.consumer.configured",
        component = "platform_billing",
        operation = "consume_billable_usage",
        max_in_flight,
        max_ack_pending = 64
    );

    loop {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(
                    event.name = "jetstream.consumer.fetch.failed",
                    component = "platform_billing",
                    operation = "consume_billable_usage",
                    dependency = "nats_jetstream",
                    error_code = "billing_usage_consumer_fetch_failed",
                    error_category = "connectivity",
                    retryable = true,
                    error_class = %error
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(Duration::from_secs(1)) => {}
                }
                continue;
            }
        };

        let mut tasks = JoinSet::new();
        let mut stream_ended = false;
        while !stream_ended {
            tokio::select! {
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        while tasks.join_next().await.is_some() {}
                        return;
                    }
                }
                result = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(Err(error)) = result {
                        tracing::error!(
                            event.name = "jetstream.consumer.task.failed",
                            component = "platform_billing",
                            operation = "process_billable_usage",
                            error_code = "billing_usage_task_join_failed",
                            error_class = %error
                        );
                        counter!("platform_billing_usage_task_failures_total").increment(1);
                    }
                }
                message = messages.next(), if tasks.len() < max_in_flight => {
                    match message {
                        Some(Ok(message)) => {
                            let service = service.clone();
                            let pool = pool.clone();
                            tasks.spawn(async move {
                                process_message(message, &service, &pool).await;
                            });
                        }
                        Some(Err(error)) => {
                            tracing::warn!(
                                event.name = "jetstream.consumer.stream.failed",
                                component = "platform_billing",
                                operation = "read_billable_usage",
                                dependency = "nats_jetstream",
                                error_code = "billing_usage_stream_failed",
                                error_category = "connectivity",
                                retryable = true,
                                error_class = %error
                            );
                            stream_ended = true;
                        }
                        None => stream_ended = true,
                    }
                }
            }
        }
        while tasks.join_next().await.is_some() {}
        if !*shutdown.borrow() {
            time::sleep(Duration::from_millis(250)).await;
        } else {
            return;
        }
    }
}

fn configured_max_in_flight() -> usize {
    match env::var("PLATFORM_BILLING_USAGE_MAX_IN_FLIGHT") {
        Ok(value) => match value.parse::<usize>() {
            Ok(value) if (1..=MAX_IN_FLIGHT_LIMIT).contains(&value) => value,
            _ => {
                tracing::warn!(
                    event.name = "service.configuration.invalid",
                    component = "platform_billing",
                    operation = "configure_usage_consumer_concurrency",
                    error_code = "billing_usage_max_in_flight_invalid",
                    fallback = DEFAULT_MAX_IN_FLIGHT
                );
                DEFAULT_MAX_IN_FLIGHT
            }
        },
        Err(_) => DEFAULT_MAX_IN_FLIGHT,
    }
}
