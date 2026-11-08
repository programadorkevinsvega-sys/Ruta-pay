use chrono::Utc;
use platform_billing::{
    application::{BillingService, RecordUsage},
    infrastructure::inbox::{self, InboxDisposition},
};
use serde_json::json;
use uuid::Uuid;

use super::super::database::TestDatabase;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_inbox_completion_rolls_back_usage_and_allows_safe_recovery() {
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
        "correlation_id": "usage-recovery-rollback",
        "payload": {"tenant_id":tenant_id,"usage_id":usage_id}
    });
    let inbox_id = match inbox::receive(
        &database.runtime_pool,
        tenant_id,
        "entitlements_quotas",
        event_id,
        "billing.usage.recorded",
        1,
        &envelope,
    )
    .await
    .expect("inbox receipt persists") {
        InboxDisposition::Inserted(id) => id,
        _ => panic!("first receipt must be inserted"),
    };
    let source_event_id = usage_id.to_string();
    let command = || RecordUsage {
        tenant_id,
        meter_key: "api.requests",
        source_system: "entitlements_quotas",
        source_event_id: &source_event_id,
        quantity: 7,
        unit: "request",
        occurred_at,
        attributes: json!({"environment":"test"}),
    };

    assert!(
        service
            .record_usage_with_inbox(command(), Uuid::new_v4())
            .await
            .is_err(),
        "a missing inbox row must abort the whole transaction"
    );
    let before_retry: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_billing.usage_events WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=$3",
    )
    .bind(tenant_id)
    .bind("entitlements_quotas")
    .bind(&source_event_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("rolled back usage is inspectable");
    assert_eq!(before_retry, 0, "usage insert was rolled back");

    service
        .record_usage_with_inbox(command(), inbox_id)
        .await
        .expect("retry persists usage and marks the original inbox receipt");
    let after_retry: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_billing.usage_events WHERE tenant_id=$1 AND source_system=$2 AND source_event_id=$3",
    )
    .bind(tenant_id)
    .bind("entitlements_quotas")
    .bind(&source_event_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("recovered usage is inspectable");
    assert_eq!(after_retry, 1);
    let disposition: String = sqlx::query_scalar(
        "SELECT disposition FROM platform_billing.inbox_events WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant_id)
    .bind(inbox_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("recovered inbox state is inspectable");
    assert_eq!(disposition, "processed");
    database.finish().await;
}
