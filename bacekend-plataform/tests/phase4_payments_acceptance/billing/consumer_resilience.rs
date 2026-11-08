use async_nats::jetstream::{
    self,
    consumer::{AckPolicy, DeliverPolicy},
};
use chrono::{Duration, Utc};
use platform_runtime::{
    event_topology::{delete_e2e_stream, EventTopology},
    event_workers::run_with_topology,
};
use serde_json::{json, Value};
use sqlx::Row;
use std::time::{Duration as StdDuration, Instant};
use tokio::{sync::watch, time};
use uuid::Uuid;

use super::super::database::TestDatabase;

const LOAD_EVENTS: usize = 20_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn billing_usage_consumer_survives_multiple_replicas_load_duplicates_and_database_outage() {
    let database = TestDatabase::new().await;
    let replica_pools = [
        database.runtime_pool.clone(),
        database.additional_runtime_pool().await,
    ];
    let namespace = format!("e2e_usage_load_{}", Uuid::new_v4().simple());
    let topology = EventTopology::from_namespace(Some(&namespace)).expect("isolated topology");
    let nats_url = std::env::var("PLATFORM_NATS_URL").expect("CI provides NATS URL");
    let nats_user = std::env::var("NATS_USER").expect("CI provides NATS user");
    let nats_password = std::env::var("NATS_PASSWORD").expect("CI provides NATS password");
    let redis_url = std::env::var("PLATFORM_REDIS_URL").expect("CI provides Redis URL");
    let client = async_nats::ConnectOptions::new()
        .user_and_password(nats_user.clone(), nats_password.clone())
        .connect(&nats_url)
        .await
        .expect("authenticated JetStream connection");
    let jetstream = jetstream::new(client);
    let stream = topology.ensure_stream(&jetstream).await.expect("stream created");
    let subject = format!("{}.billing.usage.recorded", topology.subject_prefix);
    let consumer = stream
        .get_or_create_consumer(
            &topology.billing_consumer,
            jetstream::consumer::pull::Config {
                durable_name: Some(topology.billing_consumer.clone()),
                filter_subject: subject.clone(),
                deliver_policy: DeliverPolicy::All,
                ack_policy: AckPolicy::Explicit,
                ack_wait: StdDuration::from_secs(120),
                max_deliver: -1,
                max_ack_pending: 512,
                ..Default::default()
            },
        )
        .await
        .expect("shared durable billing consumer created");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut replicas = Vec::new();
    for pool in replica_pools.iter().cloned() {
        replicas.push(tokio::spawn(run_with_topology(
            pool,
            redis_url.clone(),
            nats_url.clone(),
            nats_user.clone(),
            nats_password.clone(),
            shutdown_rx.clone(),
            topology.clone(),
        )));
    }

    let tenants = (0..4).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
    let first_event = usage_envelope(tenants[0], Uuid::new_v4(), Uuid::new_v4());
    let started = Instant::now();
    for batch_start in (0..LOAD_EVENTS).step_by(256) {
        let batch_end = (batch_start + 256).min(LOAD_EVENTS);
        let mut publishers = tokio::task::JoinSet::new();
        for index in batch_start..batch_end {
            let tenant = tenants[index % tenants.len()];
            let envelope = if index == 0 {
                first_event.clone()
            } else {
                usage_envelope(tenant, Uuid::new_v4(), Uuid::new_v4())
            };
            let jetstream = jetstream.clone();
            let subject = subject.clone();
            publishers.spawn(async move {
                publish(&jetstream, &subject, envelope).await;
            });
        }
        while let Some(result) = publishers.join_next().await {
            result.expect("bounded concurrent publisher task completes");
        }
    }
    wait_for_usage_count(&database.migration_pool, LOAD_EVENTS as i64).await;
    let distinct_tenants: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT tenant_id) FROM platform_billing.usage_events",
    )
    .fetch_one(&database.migration_pool)
    .await
    .expect("tenant distribution is measurable");
    assert_eq!(distinct_tenants, tenants.len() as i64, "load is persisted across all tenants");

    let elapsed = started.elapsed();
    let throughput = LOAD_EVENTS as f64 / elapsed.as_secs_f64();
    let latency = sqlx::query(
        "SELECT percentile_cont(0.50) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM recorded_at-occurred_at)) AS p50, percentile_cont(0.95) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM recorded_at-occurred_at)) AS p95, percentile_cont(0.99) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM recorded_at-occurred_at)) AS p99 FROM platform_billing.usage_events",
    )
    .fetch_one(&database.migration_pool)
    .await
    .expect("latency percentiles are measurable");
    let p50: f64 = latency.try_get("p50").expect("p50 latency");
    let p95: f64 = latency.try_get("p95").expect("p95 latency");
    let p99: f64 = latency.try_get("p99").expect("p99 latency");
    assert!(throughput.is_finite() && throughput > 0.0);
    assert!(p50.is_finite() && p95 >= p50 && p99 >= p95);
    println!(
        "billing_usage_load events={} replicas=2 throughput_events_per_second={:.2} latency_p50_seconds={:.6} latency_p95_seconds={:.6} latency_p99_seconds={:.6}",
        LOAD_EVENTS, throughput, p50, p95, p99
    );

    publish(&jetstream, &subject, first_event).await;
    time::sleep(StdDuration::from_secs(1)).await;
    assert_eq!(
        usage_count(&database.migration_pool)
            .await
            .expect("usage count is readable after duplicate delivery"),
        LOAD_EVENTS as i64,
        "duplicate event delivery cannot create a second financial usage row"
    );

    let mut conflicting_replay = first_event.clone();
    conflicting_replay["payload"]["quantity"] = json!(2);
    publish(&jetstream, &subject, conflicting_replay).await;
    time::sleep(StdDuration::from_secs(1)).await;
    assert_eq!(
        usage_count(&database.migration_pool)
            .await
            .expect("usage count is readable after conflicting replay"),
        LOAD_EVENTS as i64,
        "same inbox event ID with a different payload must not alter usage"
    );

    let recovery_event = usage_envelope(tenants[1], Uuid::new_v4(), Uuid::new_v4());
    let recovery_started = Instant::now();
    let outage_mode = if let Ok(container_id) = std::env::var("PLATFORM_TEST_POSTGRES_CONTAINER_ID") {
        database_outage_with_container(&container_id, &jetstream, &subject, recovery_event).await;
        "postgres_container_restart"
    } else {
        database.set_runtime_database_available(false).await;
        publish(&jetstream, &subject, recovery_event).await;
        time::sleep(StdDuration::from_secs(7)).await;
        assert_eq!(
            usage_count(&database.migration_pool)
                .await
                .expect("usage count is readable during connection-level outage"),
            LOAD_EVENTS as i64,
            "usage must not be persisted while runtime database access is revoked"
        );
        database.set_runtime_database_available(true).await;
        "runtime_connection_revocation"
    };
    wait_for_usage_count(&database.migration_pool, LOAD_EVENTS as i64 + 1).await;
    println!(
        "billing_usage_recovery outage_mode={} replicas=2 recovered_events=1 recovery_seconds={:.3} duplicate_deliveries=1 unique_usage_records={}",
        outage_mode,
        recovery_started.elapsed().as_secs_f64(),
        LOAD_EVENTS + 1
    );

    let total_usage = usage_count(&database.migration_pool)
        .await
        .expect("usage count is readable after database recovery");
    let total_inbox: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_billing.inbox_events WHERE producer_id='entitlements_quotas'",
    )
    .fetch_one(&database.migration_pool)
    .await
    .expect("inbox records are inspectable");
    let processed_inbox: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_billing.inbox_events WHERE producer_id='entitlements_quotas' AND disposition='processed'",
    )
    .fetch_one(&database.migration_pool)
    .await
    .expect("processed inbox records are inspectable");
    assert_eq!(total_usage, LOAD_EVENTS as i64 + 1);
    assert_eq!(total_inbox, LOAD_EVENTS as i64 + 1);
    assert_eq!(processed_inbox, total_inbox, "all accepted events complete their inbox receipt");

    shutdown_tx.send(true).expect("replica shutdown signal is delivered");
    for replica in replicas {
        time::timeout(StdDuration::from_secs(10), replica)
            .await
            .expect("replica shuts down promptly")
            .expect("replica task joins");
    }
    for pool in replica_pools.iter().skip(1) {
        pool.close().await;
    }
    drop(consumer);
    delete_e2e_stream(&jetstream, &namespace)
        .await
        .expect("isolated usage stream is deleted");
    database.finish().await;
}

async fn database_outage_with_container(
    container_id: &str,
    jetstream: &jetstream::Context,
    subject: &str,
    recovery_event: Value,
) {
    docker_container_action(container_id, "stop").await;
    publish(jetstream, subject, recovery_event).await;
    time::sleep(StdDuration::from_secs(7)).await;
    docker_container_action(container_id, "start").await;
}

async fn docker_container_action(container_id: &str, action: &'static str) {
    let container_id = container_id.to_owned();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("docker")
            .args([action, container_id.as_str()])
            .output()
    })
    .await
    .expect("Docker control task joins")
    .expect("Docker CLI executes");
    assert!(
        output.status.success(),
        "docker {action} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn usage_envelope(tenant_id: Uuid, event_id: Uuid, usage_id: Uuid) -> Value {
    let now = Utc::now();
    json!({
        "event_id": event_id,
        "event_type": "billing.usage.recorded",
        "schema_version": 1,
        "correlation_id": format!("billing-load-{}", event_id.simple()),
        "payload": {
            "tenant_id": tenant_id,
            "usage_id": usage_id,
            "price_version_id": Uuid::new_v4(),
            "meter_key": "api.requests",
            "quantity": 1,
            "unit": "request",
            "occurred_at": now.to_rfc3339(),
            "environment": "test",
            "period_start": (now - Duration::days(1)).to_rfc3339(),
            "period_end": (now + Duration::days(1)).to_rfc3339()
        }
    })
}

async fn publish(jetstream: &jetstream::Context, subject: &str, envelope: Value) {
    let payload = serde_json::to_vec(&envelope).expect("usage envelope serializes");
    let acknowledgement = jetstream
        .publish(subject.to_owned(), payload.into())
        .await
        .expect("JetStream accepts usage event");
    acknowledgement.await.expect("JetStream confirms durable publish");
}

async fn usage_count(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM platform_billing.usage_events WHERE source_system='entitlements_quotas'",
    )
    .fetch_one(pool)
    .await
}

async fn wait_for_usage_count(pool: &sqlx::PgPool, expected: i64) {
    time::timeout(StdDuration::from_secs(180), async {
        loop {
            if matches!(usage_count(pool).await, Ok(count) if count == expected) {
                return;
            }
            time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("consumer eventually persists the expected number of unique usage events");
}
