use async_nats::jetstream;
use metrics::{counter, histogram};
use platform_billing::application::BillingService;
use sqlx::PgPool;
use std::{env, fs, time::{Duration, Instant}};
use tokio::{sync::watch, time};
use tokio_stream::StreamExt;

mod processing;
use processing::process_batch;

const DEFAULT_BATCH_SIZE: usize = 128;
const MAX_BATCH_SIZE: usize = 512;
const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

pub(super) async fn run_billing_usage_consumer(
    pool: PgPool,
    consumer: jetstream::consumer::PullConsumer,
    mut shutdown: watch::Receiver<bool>,
) {
    let service = BillingService::new(pool.clone());
    tracing::info!(
        event.name = "jetstream.consumer.configured",
        component = "platform_billing",
        operation = "consume_billable_usage",
        configured_batch_size = configured_batch_size(),
        max_ack_pending = 512
    );

    loop {
        if *shutdown.borrow() { return; }
        let batch_limit = adaptive_batch_limit(&pool);
        let fetch_started = Instant::now();
        let fetch = consumer.fetch()
            .max_messages(batch_limit)
            .max_bytes(MAX_BATCH_BYTES)
            .expires(Duration::from_millis(500))
            .messages();

        let mut messages = tokio::select! {
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() { return; }
                continue;
            }
            result = fetch => match result {
                Ok(messages) => messages,
                Err(error) => {
                    tracing::warn!(
                        event.name = "jetstream.consumer.fetch.failed",
                        component = "platform_billing",
                        operation = "fetch_billable_usage_batch",
                        dependency = "nats_jetstream",
                        error_code = "billing_usage_consumer_fetch_failed",
                        error_category = "connectivity",
                        retryable = true,
                        error_class = %error
                    );
                    counter!("platform_billing_usage_fetch_failures_total").increment(1);
                    time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            }
        };

        let mut batch = Vec::with_capacity(batch_limit);
        while let Some(message) = messages.next().await {
            match message {
                Ok(message) => batch.push(message),
                Err(error) => {
                    tracing::warn!(
                        event.name = "jetstream.consumer.stream.failed",
                        component = "platform_billing",
                        operation = "read_billable_usage_batch",
                        dependency = "nats_jetstream",
                        error_code = "billing_usage_stream_failed",
                        error_category = "connectivity",
                        retryable = true,
                        error_class = %error
                    );
                    counter!("platform_billing_usage_stream_failures_total").increment(1);
                    break;
                }
            }
        }
        histogram!("platform_billing_usage_fetch_wait_seconds")
            .record(fetch_started.elapsed().as_secs_f64());
        if batch.is_empty() {
            continue;
        }

        histogram!("platform_billing_usage_batch_size").record(batch.len() as f64);
        counter!("platform_billing_usage_batches_total").increment(1);
        let process_started = Instant::now();
        process_batch(batch, &service).await;
        histogram!("platform_billing_usage_batch_processing_seconds")
            .record(process_started.elapsed().as_secs_f64());
    }
}

fn configured_batch_size() -> usize {
    match env::var("PLATFORM_BILLING_USAGE_BATCH_SIZE") {
        Ok(value) => match value.parse::<usize>() {
            Ok(value) if (1..=MAX_BATCH_SIZE).contains(&value) => value,
            _ => {
                tracing::warn!(
                    event.name = "service.configuration.invalid",
                    component = "platform_billing",
                    operation = "configure_usage_batch_size",
                    error_code = "billing_usage_batch_size_invalid",
                    fallback = DEFAULT_BATCH_SIZE
                );
                DEFAULT_BATCH_SIZE
            }
        },
        Err(_) => DEFAULT_BATCH_SIZE,
    }
}

/// Bounds fetch size using currently idle database connections and cgroup
/// CPU/memory pressure. The bound is recalculated before every pull.
fn adaptive_batch_limit(pool: &PgPool) -> usize {
    let idle_connections = pool.num_idle();
    let mut limit = configured_batch_size().min(idle_connections.saturating_mul(32).max(1));

    let single_cpu = fs::read_to_string("/sys/fs/cgroup/cpu.max")
        .ok()
        .and_then(|contents| {
            let mut fields = contents.split_whitespace();
            let quota = fields.next()?;
            if quota == "max" { return None; }
            let quota = quota.parse::<f64>().ok()?;
            let period = fields.next()?.parse::<f64>().ok()?;
            Some(period > 0.0 && quota / period <= 1.0)
        })
        .unwrap_or(false);
    if single_cpu {
        limit = limit.min(64);
    }

    let memory_pressure = fs::read_to_string("/sys/fs/cgroup/memory.current")
        .ok()
        .zip(fs::read_to_string("/sys/fs/cgroup/memory.max").ok())
        .and_then(|(current, maximum)| {
            let current = current.trim().parse::<u64>().ok()?;
            let maximum = maximum.trim().parse::<u64>().ok()?;
            Some(maximum > 0 && current.saturating_mul(100) / maximum >= 85)
        })
        .unwrap_or(false);
    if memory_pressure {
        limit = (limit / 2).max(1);
    }

    limit.clamp(1, MAX_BATCH_SIZE)
}
