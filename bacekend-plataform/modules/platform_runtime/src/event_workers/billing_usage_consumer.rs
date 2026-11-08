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
const BACKPRESSURE_POLL: Duration = Duration::from_millis(100);

#[derive(Default)]
struct AdaptiveBackpressure {
    previous_cpu: Option<(u64, Instant)>,
}

impl AdaptiveBackpressure {
    fn batch_limit(&mut self, pool: &PgPool) -> Option<usize> {
        let idle = pool.num_idle();
        let total = pool.size() as usize;
        let cpu = self.cpu_utilization();
        let memory = memory_utilization();
        let limit = pressure_limited_batch(
            configured_batch_size(),
            idle,
            total,
            cpu,
            memory,
        );
        if let Some(limit) = limit {
            histogram!("platform_billing_usage_adaptive_batch_limit").record(limit as f64);
        } else {
            counter!("platform_billing_usage_backpressure_pauses_total", "reason" => "pool_exhausted").increment(1);
        }
        if cpu.is_some_and(|value| value >= 0.75) {
            counter!("platform_billing_usage_resource_pressure_total", "resource" => "cpu").increment(1);
        }
        if memory.is_some_and(|value| value >= 0.85) {
            counter!("platform_billing_usage_resource_pressure_total", "resource" => "memory").increment(1);
        }
        limit
    }

    fn cpu_utilization(&mut self) -> Option<f64> {
        let quota_cores = cpu_quota_cores()?;
        let usage = fs::read_to_string("/sys/fs/cgroup/cpu.stat")
            .ok()?
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                (fields.next()? == "usage_usec").then(|| fields.next()?.parse::<u64>().ok()).flatten()
            })?;
        let now = Instant::now();
        let previous = self.previous_cpu.replace((usage, now));
        let (previous_usage, previous_at) = previous?;
        let elapsed_micros = now.duration_since(previous_at).as_secs_f64() * 1_000_000.0;
        if elapsed_micros <= 0.0 || usage < previous_usage {
            return None;
        }
        Some(((usage - previous_usage) as f64 / elapsed_micros / quota_cores).clamp(0.0, 1.0))
    }
}

fn pressure_limited_batch(
    configured: usize,
    idle: usize,
    total: usize,
    cpu: Option<f64>,
    memory: Option<f64>,
) -> Option<usize> {
    // A lazy SQLx pool starts at size zero. Permit a small bootstrap fetch so
    // the first persistence operation can establish its initial connection.
    if total == 0 {
        return Some(configured.min(32).clamp(1, MAX_BATCH_SIZE));
    }
    if idle == 0 {
        return None;
    }
    let pool_capacity = idle.saturating_mul(32).min(MAX_BATCH_SIZE);
    let mut limit = configured.min(pool_capacity);
    if let Some(cpu) = cpu {
        if cpu >= 0.90 {
            limit /= 2;
        } else if cpu >= 0.75 {
            limit = limit.saturating_mul(3) / 4;
        }
    }
    if let Some(memory) = memory {
        if memory >= 0.95 {
            limit /= 2;
        } else if memory >= 0.85 {
            limit = limit.saturating_mul(3) / 4;
        }
    }
    Some(limit.clamp(1, MAX_BATCH_SIZE))
}

fn cpu_quota_cores() -> Option<f64> {
    let host_capacity = std::thread::available_parallelism()
        .ok()
        .map(|value| value.get() as f64);
    let Some(contents) = fs::read_to_string("/sys/fs/cgroup/cpu.max").ok() else {
        return host_capacity;
    };
    let mut fields = contents.split_whitespace();
    let Some(quota) = fields.next() else { return host_capacity; };
    if quota == "max" {
        return host_capacity;
    }
    let quota = quota.parse::<f64>().ok()?;
    let period = fields.next()?.parse::<f64>().ok()?;
    (period > 0.0).then_some(quota / period)
}

fn memory_utilization() -> Option<f64> {
    let current = fs::read_to_string("/sys/fs/cgroup/memory.current").ok()?
        .trim().parse::<u64>().ok()?;
    let maximum = fs::read_to_string("/sys/fs/cgroup/memory.max").ok()?
        .trim().parse::<u64>().ok()?;
    (maximum > 0).then_some((current as f64 / maximum as f64).clamp(0.0, 1.0))
}

pub(super) async fn run_billing_usage_consumer(
    pool: PgPool,
    consumer: jetstream::consumer::PullConsumer,
    mut shutdown: watch::Receiver<bool>,
) {
    let service = BillingService::new(pool.clone());
    let mut backpressure = AdaptiveBackpressure::default();
    tracing::info!(
        event.name = "jetstream.consumer.configured",
        component = "platform_billing",
        operation = "consume_billable_usage",
        configured_batch_size = configured_batch_size(),
        max_ack_pending = 512
    );

    loop {
        if *shutdown.borrow() { return; }
        let Some(batch_limit) = backpressure.batch_limit(&pool) else {
            tokio::select! {
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() { return; }
                }
                _ = time::sleep(BACKPRESSURE_POLL) => {}
            }
            continue;
        };
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
        process_batch(batch, &service, pool.num_idle().max(1)).await;
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

#[cfg(test)]
mod tests {
    use super::pressure_limited_batch;

    #[test]
    fn pauses_fetch_when_database_pool_has_no_idle_connection() {
        assert_eq!(pressure_limited_batch(128, 0, 16, Some(0.1), Some(0.2)), None);
    }

    #[test]
    fn permits_a_bounded_bootstrap_fetch_for_a_lazy_pool() {
        assert_eq!(pressure_limited_batch(128, 0, 0, None, None), Some(32));
    }

    #[test]
    fn scales_batch_with_pool_capacity_and_cpu_pressure() {
        assert_eq!(pressure_limited_batch(128, 2, 16, Some(0.1), Some(0.2)), Some(64));
        assert_eq!(pressure_limited_batch(128, 8, 16, Some(0.80), Some(0.2)), Some(96));
        assert_eq!(pressure_limited_batch(128, 8, 16, Some(0.95), Some(0.2)), Some(64));
    }

    #[test]
    fn applies_memory_pressure_after_cpu_pressure() {
        assert_eq!(pressure_limited_batch(128, 8, 16, Some(0.80), Some(0.90)), Some(72));
        assert_eq!(pressure_limited_batch(128, 8, 16, Some(0.95), Some(0.98)), Some(32));
    }

    #[test]
    fn caps_batch_size_and_keeps_a_minimum_when_capacity_exists() {
        assert_eq!(pressure_limited_batch(512, 64, 64, Some(0.0), Some(0.0)), Some(512));
        assert_eq!(pressure_limited_batch(1, 1, 1, Some(1.0), Some(1.0)), Some(1));
    }
}
