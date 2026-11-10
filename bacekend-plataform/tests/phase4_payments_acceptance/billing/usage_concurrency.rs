use chrono::Utc;
use platform_billing::{
    application::{BillingService, RecordUsage},
    infrastructure::inbox::{self, InboxDisposition},
};
use serde_json::json;
use uuid::Uuid;

use super::super::database::TestDatabase;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn usage_inbox_is_atomic_and_concurrent_redelivery_is_idempotent() {
    let database = TestDatabase::new().await;
    let service = BillingService::new(database.runtime_pool.clone());
    let tenant_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let usage_id = Uuid::new_v4();
    let occurred_at = Utc::now();
    let envelope = json!({
        "event_id": event_id,
        "event_type": "billing.usage.recorded",
        "schema_version": 1,
        "correlation_id": "usage-recovery-acceptance",
        "payload": {
            "tenant_id": tenant_id,
            "usage_id": usage_id,
            "price_version_id": Uuid::new_v4(),
            "meter_key": "api.requests",
            "quantity": 42,
            "unit": "request",
            "occurred_at": occurred_at.to_rfc3339(),
            "environment": "test",
            "period_start": (occurred_at - chrono::Duration::days(1)).to_rfc3339(),
            "period_end": (occurred_at + chrono::Duration::days(1)).to_rfc3339()
        }
    });

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let service = service.clone();
        let pool = database.runtime_pool.clone();
        let envelope = envelope.clone();
        tasks.spawn(async move {
            let disposition = inbox::receive(
                &pool,
                tenant_id,
                "entitlements_quotas",
                event_id,
                "billing.usage.recorded",
                1,
                &envelope,
            )
            .await
            .expect("concurrent inbox receipt is idempotent");
            let inbox_id = match disposition {
                InboxDisposition::Inserted(id)
                | InboxDisposition::Duplicate {
                    id,
                    processed: false,
                } => id,
                InboxDisposition::Duplicate {
                    processed: true, ..
                } => return None,
            };
            let source_event_id = usage_id.to_string();
            let command = RecordUsage {
                tenant_id,
                meter_key: "api.requests",
                source_system: "entitlements_quotas",
                source_event_id: &source_event_id,
                quantity: 42,
                unit: "request",
                occurred_at,
                attributes: json!({"environment":"test"}),
            };
            Some(
                service
                    .record_usage_with_inbox(command, inbox_id)
                    .await
                    .expect("usage and inbox completion commit atomically"),
            )
        });
    }

    let mut usage_ids = Vec::new();
    let mut original_writes = 0;
    while let Some(result) = tasks.join_next().await {
        if let Some((id, duplicate)) = result.expect("concurrent worker completes") {
            usage_ids.push(id);
            if !duplicate {
                original_writes += 1;
            }
        }
    }
    assert_eq!(
        original_writes, 1,
        "one transaction inserts the usage event"
    );
    assert!(!usage_ids.is_empty());
    assert!(usage_ids.iter().all(|id| *id == usage_ids[0]));

    let usage_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_billing.usage_events WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=$3",
    )
    .bind(tenant_id)
    .bind("entitlements_quotas")
    .bind(usage_id.to_string())
    .fetch_one(&database.migration_pool)
    .await
    .expect("usage count is inspectable");
    assert_eq!(usage_count, 1);

    let disposition: String = sqlx::query_scalar(
        "SELECT disposition FROM platform_billing.inbox_events WHERE tenant_id=$1 AND producer_id=$2 AND event_id=$3",
    )
    .bind(tenant_id)
    .bind("entitlements_quotas")
    .bind(event_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("inbox disposition is inspectable");
    assert_eq!(disposition, "processed");

    database.finish().await;
}
