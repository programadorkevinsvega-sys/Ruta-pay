mod billing_usage_consumer;
mod consumers;
mod membership_consumer;
mod realtime_consumer;
mod relays;

use async_nats::{ConnectOptions, jetstream};
use sqlx::PgPool;
use std::{env, path::PathBuf, time::Duration};
use tokio::{sync::watch, time};

use self::{
    billing_usage_consumer::run_billing_usage_consumer,
    consumers::{
        ensure_billing_usage_consumer, ensure_membership_consumer, ensure_realtime_consumer,
    },
    membership_consumer::run_membership_consumer,
    realtime_consumer::run_realtime_consumer,
    relays::run_relays,
};
use crate::event_topology::EventTopology;

#[derive(Clone, Debug)]
pub enum NatsAuthentication {
    UserPassword { username: String, password: String },
    CertificateMapped,
}

pub async fn run(
    pool: PgPool,
    redis_client: redis::Client,
    url: String,
    username: String,
    password: String,
    shutdown: watch::Receiver<bool>,
) {
    let certificate_mapped = env::var("APP_ENV")
        .is_ok_and(|value| value.eq_ignore_ascii_case("production"))
        || (env::var("PAYVIA_REQUIRE_MTLS").is_ok_and(|value| value.eq_ignore_ascii_case("true"))
            && username.trim().is_empty()
            && password.is_empty());
    let authentication = if certificate_mapped {
        NatsAuthentication::CertificateMapped
    } else {
        NatsAuthentication::UserPassword { username, password }
    };
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
    run_with_topology(pool, redis_client, url, authentication, shutdown, topology).await;
}

/// Runs a complete event runtime against an explicit, isolated topology.
/// This also allows integration tests to exercise multiple replicas sharing
/// the same durable JetStream consumers without mutating process environment.
pub async fn run_with_topology(
    pool: PgPool,
    redis_client: redis::Client,
    url: String,
    authentication: NatsAuthentication,
    mut shutdown: watch::Receiver<bool>,
    topology: EventTopology,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let options = match build_nats_options(authentication.clone()) {
            Ok(options) => options,
            Err(error_code) => {
                tracing::error!(
                    event.name = "service.configuration.invalid",
                    component = "platform_event_runtime",
                    operation = "configure_nats_mtls",
                    error_code,
                    error_category = "configuration",
                    retryable = false
                );
                tokio::select! {
                    result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; },
                    _ = time::sleep(backoff) => {}
                }
                continue;
            }
        };
        let client = match options.connect(url.as_str()).await {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_event_runtime",
                    operation = "connect_nats",
                    dependency = "nats_jetstream",
                    error_code = "nats_connect_failed",
                    error_category = "connectivity",
                    retryable = true,
                    error_class = %error
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
        let billing_subject = format!("{}.billing.usage.recorded", topology.subject_prefix);
        let billing_consumer = match ensure_billing_usage_consumer(
            &stream,
            &topology.billing_consumer,
            &billing_subject,
        )
        .await
        {
            Ok(consumer) => consumer,
            Err(()) => {
                tracing::error!(
                    event.name = "service.startup.dependency.failed",
                    component = "platform_event_runtime",
                    operation = "ensure_billing_usage_consumer",
                    dependency = "nats_jetstream",
                    error_code = "jetstream_billing_consumer_unavailable",
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
            consumer = %topology.membership_consumer,
            billing_consumer = %topology.billing_consumer
        );
        let mut workers_shutdown = shutdown.clone();
        tokio::select! {
            result = workers_shutdown.changed() => if result.is_err() || *workers_shutdown.borrow() { return; },
            _ = run_relays(pool.clone(), jetstream, topology.subject_prefix.clone(), shutdown.clone()) => {},
            _ = run_membership_consumer(pool.clone(), consumer, shutdown.clone()) => {},
            _ = run_realtime_consumer(pool.clone(), redis_client.clone(), realtime_consumer, shutdown.clone()) => {},
            _ = run_billing_usage_consumer(pool.clone(), billing_consumer, shutdown.clone()) => {},
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

fn build_nats_options(authentication: NatsAuthentication) -> Result<ConnectOptions, &'static str> {
    let certificate_mapped = matches!(&authentication, NatsAuthentication::CertificateMapped);
    let environment = env::var("APP_ENV").unwrap_or_else(|_| "development".to_owned());
    let secure_environment = matches!(
        environment.to_ascii_lowercase().as_str(),
        "staging" | "production"
    );
    let mtls_required =
        env::var("PAYVIA_REQUIRE_MTLS").is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    if secure_environment && !mtls_required {
        return Err("platform_nats_mtls_required_for_environment");
    }
    if environment.eq_ignore_ascii_case("production") && !certificate_mapped {
        return Err("platform_nats_certificate_identity_required");
    }
    let tls_enabled = mtls_required || certificate_mapped;
    let root = tls_enabled
        .then(|| env::var_os("NATS_TLS_ROOT_CA").filter(|value| !value.is_empty()))
        .flatten();
    let certificate = tls_enabled
        .then(|| env::var_os("NATS_TLS_CLIENT_CERT").filter(|value| !value.is_empty()))
        .flatten();
    let private_key = tls_enabled
        .then(|| env::var_os("NATS_TLS_CLIENT_KEY").filter(|value| !value.is_empty()))
        .flatten();
    match (root, certificate, private_key, authentication) {
        (
            Some(root),
            Some(certificate),
            Some(private_key),
            NatsAuthentication::CertificateMapped,
        ) => Ok(ConnectOptions::new()
            .add_root_certificates(PathBuf::from(root))
            .add_client_certificate(PathBuf::from(certificate), PathBuf::from(private_key))
            .require_tls(true)),
        (
            Some(root),
            Some(certificate),
            Some(private_key),
            NatsAuthentication::UserPassword { username, password },
        ) => Ok(ConnectOptions::with_user_and_password(username, password)
            .add_root_certificates(PathBuf::from(root))
            .add_client_certificate(PathBuf::from(certificate), PathBuf::from(private_key))
            .require_tls(true)),
        (None, None, None, NatsAuthentication::UserPassword { username, password })
            if !tls_enabled =>
        {
            Ok(ConnectOptions::with_user_and_password(username, password))
        }
        (None, None, None, _) => Err("platform_nats_mtls_required"),
        _ => Err("platform_nats_mtls_configuration_incomplete"),
    }
}
