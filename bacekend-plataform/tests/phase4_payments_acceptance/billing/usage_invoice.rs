use chrono::{Duration, Utc};
use platform_billing::application::{BillingService, CreateInvoice, InvoiceLine, RecordUsage};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::super::database::TestDatabase;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn usage_invoice_snapshot_ledger_and_financial_immutability_are_enforced() {
    let db = TestDatabase::new().await;
    let service = BillingService::new(db.runtime_pool.clone());
    let tenant = Uuid::new_v4();
    let at = Utc::now();
    let usage = RecordUsage {
        tenant_id: tenant,
        meter_key: "api.requests",
        source_system: "platform_runtime",
        source_event_id: "usage-0001",
        quantity: 120,
        unit: "request",
        occurred_at: at,
        attributes: json!({"region":"co"}),
    };
    let (usage_id, replayed) = service.record_usage(usage).await.expect("usage persists");
    assert!(!replayed);
    let replay = RecordUsage {
        tenant_id: tenant,
        meter_key: "api.requests",
        source_system: "platform_runtime",
        source_event_id: "usage-0001",
        quantity: 120,
        unit: "request",
        occurred_at: at,
        attributes: json!({"region":"co"}),
    };
    let (same_id, replayed) = service
        .record_usage(replay)
        .await
        .expect("usage replay is idempotent");
    assert!(replayed);
    assert_eq!(usage_id, same_id);
    let conflict = RecordUsage {
        tenant_id: tenant,
        meter_key: "api.requests",
        source_system: "platform_runtime",
        source_event_id: "usage-0001",
        quantity: 121,
        unit: "request",
        occurred_at: at,
        attributes: json!({"region":"co"}),
    };
    assert!(service.record_usage(conflict).await.is_err());

    let invoice = service
        .create_draft(CreateInvoice {
            tenant_id: tenant,
            invoice_number: "PV-INT-0001",
            currency: "COP",
            period_start: at - Duration::days(30),
            period_end: at,
            due_at: None,
            correlation_id: "billing-test",
        })
        .await
        .expect("draft invoice created");
    service
        .add_line(
            tenant,
            invoice,
            InvoiceLine {
                description: "API requests",
                meter_key: Some("api.requests"),
                source_usage_event_id: Some(usage_id),
                quantity: 120,
                unit: "request",
                unit_price_minor: 3,
            },
        )
        .await
        .expect("priced usage line is attached");
    let hash = service
        .issue_invoice(tenant, invoice, "billing-test")
        .await
        .expect("invoice issues atomically");
    let replay_hash = service
        .issue_invoice(tenant, invoice, "billing-test-replay")
        .await
        .expect("invoice issue replay returns original snapshot");
    assert_eq!(
        hash, replay_hash,
        "replay cannot duplicate invoice accounting or events"
    );
    let usage_summary = service
        .summarize_usage(
            tenant,
            "api.requests",
            "request",
            at - Duration::days(1),
            at + Duration::days(1),
        )
        .await
        .expect("usage aggregates for a bounded period");
    assert_eq!(usage_summary.quantity, 120);
    assert_eq!(usage_summary.event_count, 1);
    let snapshot: serde_json::Value = sqlx::query_scalar(
        "SELECT snapshot FROM platform_billing.invoices WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant)
    .bind(invoice)
    .fetch_one(&db.migration_pool)
    .await
    .expect("snapshot exists");
    let stored_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT snapshot_sha256 FROM platform_billing.invoices WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant)
    .bind(invoice)
    .fetch_one(&db.migration_pool)
    .await
    .expect("snapshot hash exists");
    let actual: [u8; 32] =
        Sha256::digest(serde_json::to_vec(&snapshot).expect("snapshot serializes")).into();
    assert_eq!(hash.as_slice(), stored_hash.as_slice());
    assert_eq!(actual, hash);
    assert_eq!(snapshot["total_minor"], json!(360));
    let balanced:bool=sqlx::query_scalar("SELECT sum(amount_minor) FILTER(WHERE direction='debit')=sum(amount_minor) FILTER(WHERE direction='credit') FROM platform_billing.ledger_entries WHERE tenant_id=$1 AND journal_id=(SELECT id FROM platform_billing.ledger_journals WHERE tenant_id=$1 AND invoice_id=$2)")
        .bind(tenant).bind(invoice).fetch_one(&db.migration_pool).await.expect("ledger balance is queryable");
    assert!(balanced);
    let event_count:i64=sqlx::query_scalar("SELECT count(*) FROM platform_billing.outbox_events WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='billing.invoice.issued'")
        .bind(tenant).bind(invoice).fetch_one(&db.migration_pool).await.expect("invoice event count is readable");
    assert_eq!(
        event_count, 1,
        "invoice issuance and event are atomic and idempotent"
    );
    assert!(sqlx::query("UPDATE platform_billing.invoice_lines SET description='tampered' WHERE tenant_id=$1 AND invoice_id=$2")
        .bind(tenant).bind(invoice).execute(&db.migration_pool).await.is_err());
    assert!(
        sqlx::query(
            "UPDATE platform_billing.invoices SET total_minor=1 WHERE tenant_id=$1 AND id=$2"
        )
        .bind(tenant)
        .bind(invoice)
        .execute(&db.migration_pool)
        .await
        .is_err()
    );
    db.finish().await;
}
