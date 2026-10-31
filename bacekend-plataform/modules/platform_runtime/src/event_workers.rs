use crate::event_topology::EventTopology;
use async_nats::{
    ConnectOptions,
    jetstream::{self, consumer::AckPolicy},
};
use platform_catalog_pricing::infrastructure::outbox_relay as catalog_outbox;
use platform_entitlements_quotas::infrastructure::outbox_relay as entitlements_outbox;
use platform_identity_access::infrastructure::outbox_relay as identity_outbox;
use platform_identity_access::infrastructure::tenant_membership_consumer::{
    TenantMembershipConsumerError, consume_tenant_registered,
};
use platform_tenant_registry::infrastructure::outbox_relay as tenant_outbox;
use sqlx::PgPool;
use std::{env, time::Duration};
use tokio::{sync::watch, time};
use tokio_stream::StreamExt;

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

async fn ensure_realtime_consumer(
    stream: &jetstream::stream::Stream,
    name: &str,
    filter_subject: &str,
) -> Result<jetstream::consumer::PullConsumer, ()> {
    stream
        .get_or_create_consumer(
            name,
            jetstream::consumer::pull::Config {
                durable_name: Some(name.to_owned()),
                filter_subject: filter_subject.to_owned(),
                deliver_policy: jetstream::consumer::DeliverPolicy::New,
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                max_deliver: 20,
                max_ack_pending: 1024,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| ())
}

async fn ensure_membership_consumer(
    stream: &jetstream::stream::Stream,
    name: &str,
    filter_subject: &str,
) -> Result<jetstream::consumer::PullConsumer, ()> {
    stream
        .get_or_create_consumer(
            name,
            jetstream::consumer::pull::Config {
                durable_name: Some(name.to_owned()),
                filter_subject: filter_subject.to_owned(),
                deliver_policy: jetstream::consumer::DeliverPolicy::New,
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                max_deliver: 20,
                max_ack_pending: 512,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| ())
}

async fn run_relays(
    pool: PgPool,
    jetstream: jetstream::Context,
    subject_prefix: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval = time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(error) = identity_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "identity_access", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
                if let Err(error) = tenant_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "tenant_registry", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
                if let Err(error) = catalog_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "catalog_pricing", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
                if let Err(error) = entitlements_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "entitlements_quotas", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
            }
            result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; }
        }
    }
}

async fn run_membership_consumer(
    pool: PgPool,
    consumer: jetstream::consumer::PullConsumer,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(_) => {
                tracing::warn!(
                    event.name = "jetstream.consumer.fetch.failed",
                    component = "identity_access",
                    operation = "consume_tenant_registered",
                    dependency = "nats_jetstream",
                    error_code = "consumer_fetch_failed",
                    error_category = "connectivity",
                    retryable = true
                );
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        loop {
            tokio::select! {
                result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                message = messages.next() => match message {
                    Some(Ok(message)) => {
                        match consume_tenant_registered(&pool, &message.payload).await {
                            Ok(processed) => {
                                if let Err(error) = message.ack().await {
                                    tracing::warn!(event.name = "jetstream.consumer.ack.failed", component = "identity_access", operation = "ack_tenant_registered", dependency = "nats_jetstream", error_code = "consumer_ack_failed", error_category = "connectivity", retryable = true, error_class = %error);
                                } else if processed {
                                    tracing::info!(event.name = "membership.owner.provisioned", component = "identity_access", operation = "consume_tenant_registered", result = "success");
                                }
                            }
                            Err(TenantMembershipConsumerError::InvalidEnvelope) => {
                                tracing::error!(event.name = "jetstream.consumer.message.invalid", component = "identity_access", operation = "consume_tenant_registered", dependency = "nats_jetstream", error_code = "tenant_event_invalid", error_category = "validation", retryable = false);
                                if let Err(error) = message.ack_with(jetstream::AckKind::Term).await {
                                    tracing::error!(event.name = "jetstream.consumer.term.failed", component = "identity_access", operation = "term_invalid_event", dependency = "nats_jetstream", error_code = "consumer_term_failed", error_category = "connectivity", retryable = true, error_class = %error);
                                }
                            }
                            Err(TenantMembershipConsumerError::Database(error)) => {
                                let sqlstate = error.as_database_error().and_then(|cause| cause.code());
                                let event_correlation = serde_json::from_slice::<serde_json::Value>(&message.payload)
                                    .ok()
                                    .and_then(|envelope| envelope.get("correlation_id").and_then(serde_json::Value::as_str).map(str::to_owned))
                                    .unwrap_or_else(|| "unavailable".to_owned());
                                let permanent = sqlstate.as_deref() == Some("23503");
                                tracing::error!(event.name = "jetstream.consumer.processing.failed", component = "identity_access", operation = "consume_tenant_registered", dependency = "postgresql", error_code = sqlstate.as_deref().unwrap_or("membership_persistence_failed"), error_category = "persistence", retryable = !permanent, correlation_id = %event_correlation);
                                let acknowledgement = if permanent {
                                    message.ack_with(jetstream::AckKind::Term).await
                                } else {
                                    message.ack_with(jetstream::AckKind::Nak(Some(Duration::from_secs(5)))).await
                                };
                                if let Err(ack_error) = acknowledgement {
                                    tracing::error!(event.name = "jetstream.consumer.acknowledgement.failed", component = "identity_access", operation = "acknowledge_tenant_registered_failure", dependency = "nats_jetstream", error_code = "consumer_acknowledgement_failed", error_category = "connectivity", retryable = true, correlation_id = %event_correlation, error_class = %ack_error);
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(event.name = "jetstream.consumer.stream.failed", component = "identity_access", operation = "read_tenant_registered", dependency = "nats_jetstream", error_code = "consumer_stream_failed", error_category = "connectivity", retryable = true, error_class = %error);
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}

async fn run_realtime_consumer(
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
