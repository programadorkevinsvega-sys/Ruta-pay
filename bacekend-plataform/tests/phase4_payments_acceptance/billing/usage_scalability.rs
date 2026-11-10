use async_nats::jetstream::{
    self,
    consumer::{AckPolicy, DeliverPolicy},
};
use chrono::{Duration, Utc};
use platform_runtime::{
    event_topology::{EventTopology, delete_e2e_stream},
    event_workers::run_with_topology,
};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::HashMap,
    time::{Duration as StdDuration, Instant},
};
use tokio::{sync::watch, time};
use uuid::Uuid;

use super::super::database::TestDatabase;

const TENANT_CARDINALITIES: [usize; 4] = [1, 4, 16, 64];
const EVENTS_PER_SCENARIO: usize = 2_500;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn billing_usage_scales_across_one_four_sixteen_and_sixty_four_tenants() {
    let database = TestDatabase::new().await;
    let replica_pools = [
        database.runtime_pool.clone(),
        database.additional_runtime_pool().await,
    ];
    let namespace = format!("e2e_usage_scale_{}", Uuid::new_v4().simple());
    let topology = EventTopology::from_namespace(Some(&namespace)).expect("isolated topology");
    let nats_url = std::env::var("PLATFORM_NATS_URL").expect("CI provides NATS URL");
    let nats_user = std::env::var("NATS_USER").expect("CI provides NATS user");
    let nats_password = std::env::var("NATS_PASSWORD").expect("CI provides NATS password");
    let redis_url = std::env::var("PLATFORM_REDIS_URL").expect("CI provides Redis URL");
    let redis_client =
        redis::Client::open(redis_url.as_str()).expect("CI provides a valid Redis URL");
    let client = async_nats::ConnectOptions::new()
        .user_and_password(nats_user.clone(), nats_password.clone())
        .connect(&nats_url)
        .await
        .expect("authenticated JetStream connection");
    let jetstream = jetstream::new(client);
    let stream = topology
        .ensure_stream(&jetstream)
        .await
        .expect("stream created");
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
        .expect("durable consumer created");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut replicas = Vec::new();
    for pool in replica_pools.iter().cloned() {
        replicas.push(tokio::spawn(run_with_topology(
            pool,
            redis_client.clone(),
            nats_url.clone(),
            nats_user.clone(),
            nats_password.clone(),
            shutdown_rx.clone(),
            topology.clone(),
        )));
    }

    let mut total_persisted = 0_i64;
    let mut report = Vec::new();
    for tenant_count in TENANT_CARDINALITIES {
        let tenants = (0..tenant_count)
            .map(|_| Uuid::new_v4())
            .collect::<Vec<_>>();
        let baseline = usage_count(&database.migration_pool).await;
        let commits_before = committed_transactions(&database.migration_pool).await;
        let pg_statements_before = statement_time_ms(&database.migration_pool).await;
        let started = Instant::now();
        let mut peak_pool_size = 0_u32;
        let mut peak_pool_in_use = 0_u32;
        for batch_start in (0..EVENTS_PER_SCENARIO).step_by(128) {
            let batch_end = (batch_start + 128).min(EVENTS_PER_SCENARIO);
            let mut publishers = tokio::task::JoinSet::new();
            for index in batch_start..batch_end {
                let tenant = tenants[index % tenant_count];
                let envelope = usage_envelope(tenant, Uuid::new_v4(), Uuid::new_v4());
                let jetstream = jetstream.clone();
                let subject = subject.clone();
                publishers.spawn(async move {
                    publish(&jetstream, &subject, envelope).await;
                });
            }
            while let Some(result) = publishers.join_next().await {
                result.expect("publisher task completes");
            }
            let total_pool_size = replica_pools.iter().map(sqlx::Pool::size).sum();
            let total_pool_idle: usize = replica_pools.iter().map(sqlx::Pool::num_idle).sum();
            peak_pool_size = peak_pool_size.max(total_pool_size);
            peak_pool_in_use = peak_pool_in_use
                .max(total_pool_size.saturating_sub(total_pool_idle.min(u32::MAX as usize) as u32));
        }
        wait_for_usage_count(
            &database.migration_pool,
            baseline + EVENTS_PER_SCENARIO as i64,
        )
        .await;
        let elapsed = started.elapsed();
        let tenant_rows = sqlx::query(
            "SELECT tenant_id, count(*) AS events FROM platform_billing.usage_events
             WHERE tenant_id=ANY($1) GROUP BY tenant_id",
        )
        .bind(&tenants)
        .fetch_all(&database.migration_pool)
        .await
        .expect("tenant event distribution");
        let mut distribution = HashMap::new();
        let mut tenant_event_total = 0_i64;
        for row in tenant_rows {
            let tenant: Uuid = row.try_get("tenant_id").expect("tenant id");
            let events: i64 = row.try_get("events").expect("event count");
            tenant_event_total += events;
            distribution.insert(tenant, events);
        }
        assert_eq!(tenant_event_total, EVENTS_PER_SCENARIO as i64);
        assert_eq!(
            distribution.len(),
            tenant_count,
            "all configured tenants received events"
        );
        let latency = sqlx::query(
            "SELECT percentile_cont(0.50) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM recorded_at-occurred_at)) AS p50,
             percentile_cont(0.95) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM recorded_at-occurred_at)) AS p95,
             percentile_cont(0.99) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM recorded_at-occurred_at)) AS p99
             FROM platform_billing.usage_events WHERE tenant_id=ANY($1)"
        ).bind(&tenants).fetch_one(&database.migration_pool).await.expect("global event latency percentiles");
        let p50: f64 = latency.try_get("p50").expect("p50");
        let p95: f64 = latency.try_get("p95").expect("p95");
        let p99: f64 = latency.try_get("p99").expect("p99");
        let commits = committed_transactions(&database.migration_pool).await - commits_before;
        let sql_ms = statement_time_ms(&database.migration_pool)
            .await
            .map(|after| pg_statements_before.map(|before| after - before));
        let pool_size: u32 = replica_pools.iter().map(sqlx::Pool::size).sum();
        let pool_idle: usize = replica_pools.iter().map(sqlx::Pool::num_idle).sum();
        let throughput = EVENTS_PER_SCENARIO as f64 / elapsed.as_secs_f64();
        assert!(throughput.is_finite() && throughput > 0.0);
        assert!(p50.is_finite() && p95 >= p50 && p99 >= p95);
        assert!(distribution.values().all(|count| *count > 0));
        println!(
            "billing_usage_scalability tenants={} events={} replicas=2 throughput_events_per_second={:.2} latency_p50_seconds={:.6} latency_p95_seconds={:.6} latency_p99_seconds={:.6} postgres_commits_delta={} pg_stat_statements_sql_ms={:?} pool_total_size={} pool_total_idle={} pool_peak_size={} pool_peak_in_use={} tenant_min_events={} tenant_max_events={}",
            tenant_count,
            EVENTS_PER_SCENARIO,
            throughput,
            p50,
            p95,
            p99,
            commits,
            sql_ms,
            pool_size,
            pool_idle,
            peak_pool_size,
            peak_pool_in_use,
            distribution.values().min().copied().unwrap_or_default(),
            distribution.values().max().copied().unwrap_or_default()
        );
        report.push((tenant_count, throughput, p50, p95, p99));
        total_persisted += EVENTS_PER_SCENARIO as i64;
    }

    assert_eq!(usage_count(&database.migration_pool).await, total_persisted);
    shutdown_tx.send(true).expect("shutdown signal delivered");
    for replica in replicas {
        time::timeout(StdDuration::from_secs(10), replica)
            .await
            .expect("replica shutdown timeout")
            .expect("replica task joins");
    }
    for pool in replica_pools.iter().skip(1) {
        pool.close().await;
    }
    drop(consumer);
    delete_e2e_stream(&jetstream, &namespace)
        .await
        .expect("isolated stream deleted");
    database.finish().await;
    assert_eq!(report.len(), TENANT_CARDINALITIES.len());
}

async fn usage_count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM platform_billing.usage_events WHERE source_system='entitlements_quotas'")
        .fetch_one(pool).await.expect("usage count")
}

async fn committed_transactions(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT xact_commit FROM pg_stat_database WHERE datname=current_database()")
        .fetch_one(pool)
        .await
        .expect("PostgreSQL commit counter")
}

async fn statement_time_ms(pool: &sqlx::PgPool) -> Option<f64> {
    let available: bool =
        sqlx::query_scalar("SELECT to_regclass('public.pg_stat_statements') IS NOT NULL")
            .fetch_one(pool)
            .await
            .ok()?;
    if !available {
        return None;
    }
    sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(sum(total_exec_time),0)::float8 FROM pg_stat_statements
         WHERE query ILIKE '%platform_billing.inbox_events%'
            OR query ILIKE '%platform_billing.usage_events%'",
    )
    .fetch_one(pool)
    .await
    .ok()
}

fn usage_envelope(tenant_id: Uuid, event_id: Uuid, usage_id: Uuid) -> Value {
    let now = Utc::now();
    json!({
        "event_id": event_id, "event_type": "billing.usage.recorded",
        "schema_version": 1, "correlation_id": format!("scale-{}", event_id.simple()),
        "payload": {
            "tenant_id": tenant_id, "usage_id": usage_id, "price_version_id": Uuid::new_v4(),
            "meter_key": "api.requests", "quantity": 1, "unit": "request",
            "occurred_at": now.to_rfc3339(), "environment": "test",
            "period_start": (now-Duration::days(1)).to_rfc3339(),
            "period_end": (now+Duration::days(1)).to_rfc3339()
        }
    })
}

async fn publish(jetstream: &jetstream::Context, subject: &str, envelope: Value) {
    let payload = serde_json::to_vec(&envelope).expect("usage envelope serializes");
    jetstream
        .publish(subject.to_owned(), payload.into())
        .await
        .expect("publish accepted")
        .await
        .expect("publish acknowledged");
}

async fn wait_for_usage_count(pool: &sqlx::PgPool, expected: i64) {
    time::timeout(StdDuration::from_secs(180), async {
        loop {
            if usage_count(pool).await == expected {
                return;
            }
            time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("expected usage rows persisted");
}
