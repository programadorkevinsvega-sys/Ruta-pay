mod consumers;
mod membership_consumer;
mod realtime_consumer;
mod relays;

use async_nats::{ConnectOptions, jetstream};
use sqlx::PgPool;
use std::{env, time::Duration};
use tokio::{sync::watch, time};

use self::{
    consumers::{ensure_membership_consumer, ensure_realtime_consumer},
    membership_consumer::run_membership_consumer,
    realtime_consumer::run_realtime_consumer,
    relays::run_relays,
};
use crate::event_topology::EventTopology;

pub async fn run(
    pool: PgPool,
    redis_url: String,
    url: String,
    username: String,
    password: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_secs(1);
    let namespace = env::var("PLATFORM_EVENT_NAMESPACE").ok();
    let topology = match EventTopology::from_namespace(namespace.as_deref()) {
        Ok(topology) => topology,
        Err(error_code) => {
            tracing::error!(
                event.name = "service.configuration.invalid",
                component = "platform_event_runtime",
                operation = "configure_event_topology",
                error_code,
                error_category = "configuration",
                retryable = false
            );
            return;
        }
    };
    loop {
        let client = match ConnectOptions::with_user_and_password(
            username.clone(),
            password.clone(),
        )
        .connect(url.as_str())
        .await
        {
            Ok(client) => client,
            Err(_) => {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_event_runtime",
                    operation = "connect_nats",
                    dependency = "nats_jetstream",
                    error_code = "nats_connect_failed",
                    error_category = "connectivity",
                    retryable = true
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        let jetstream = jetstream::new(client);
        let stream = match topology.ensure_stream(&jetstream).await {
            Ok(stream) => stream,
            Err(error_code) => {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_event_runtime",
                    operation = "ensure_event_stream",
                    dependency = "nats_jetstream",
                    error_code,
                    error_category = "dependency",
                    retryable = true
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        let membership_subject = format!("{}.tenant.registered", topology.subject_prefix);
        let consumer = match ensure_membership_consumer(
            &stream,
            &topology.membership_consumer,
            &membership_subject,
        )
        .await
        {
            Ok(consumer) => consumer,
            Err(()) => {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_event_runtime",
                    operation = "ensure_membership_consumer",
                    dependency = "nats_jetstream",
                    error_code = "jetstream_consumer_unavailable",
                    error_category = "dependency",
                    retryable = true
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        let realtime_filter = format!("{}.>", topology.subject_prefix);
        let realtime_consumer = match ensure_realtime_consumer(
            &stream,
            &topology.realtime_consumer,
            &realtime_filter,
        )
        .await
        {
            Ok(consumer) => consumer,
            Err(()) => {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_event_runtime",
                    operation = "ensure_realtime_consumer",
                    dependency = "nats_jetstream",
                    error_code = "jetstream_realtime_consumer_unavailable",
                    error_category = "dependency",
                    retryable = true
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        tracing::info!(
            event.name = "service.dependency.connected",
            component = "platform_event_runtime",
            dependency = "nats_jetstream",
            stream = %topology.stream_name,
            consumer = %topology.membership_consumer
        );
        let mut workers_shutdown = shutdown.clone();
        tokio::select! {
            result = workers_shutdown.changed() => if result.is_err() || *workers_shutdown.borrow() { return; },
            _ = run_relays(pool.clone(), jetstream, topology.subject_prefix.clone(), shutdown.clone()) => {},
            _ = run_membership_consumer(pool.clone(), consumer, shutdown.clone()) => {},
            _ = run_realtime_consumer(pool.clone(), redis_url.clone(), realtime_consumer, shutdown.clone()) => {},
        }
        tracing::warn!(
            event.name = "service.dependency.worker_restarting",
            component = "platform_event_runtime",
            dependency = "nats_jetstream",
            retryable = true
        );
        backoff = Duration::from_secs(1);
    }
}
