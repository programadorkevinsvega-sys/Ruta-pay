use chrono::{Duration, Utc};
use platform_billing::{
    application::{BillingService, CreateInvoice, InvoiceLine, RecordUsage},
    infrastructure::{
        collection::{CreateCollectionAttempt, create_attempt, transition_attempt},
        inbox::{InboxDisposition, receive},
        outbox,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::database::TestDatabase;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn billing_collection_inbox_outbox_concurrency_and_recovery_are_isolated() {
    let db = TestDatabase::new().await;
    let service = BillingService::new(db.runtime_pool.clone());
    let tenant = Uuid::new_v4();
    let now = Utc::now();
    let invoice = service
        .create_draft(CreateInvoice {
            tenant_id: tenant,
            invoice_number: "PV-INT-0002",
            currency: "COP",
            period_start: now - Duration::days(1),
            period_end: now,
            due_at: None,
            correlation_id: "billing-concurrency",
        })
        .await
        .expect("invoice draft exists");
    service
        .add_line(
            tenant,
            invoice,
            InvoiceLine {
                description: "Fixed SaaS plan",
                meter_key: None,
                source_usage_event_id: None,
                quantity: 1,
                unit: "month",
                unit_price_minor: 50000,
            },
        )
        .await
        .expect("approved fixed amount supplied explicitly");
    service
        .issue_invoice(tenant, invoice, "billing-concurrency")
        .await
        .expect("invoice issued");

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..10 {
        let pool = db.runtime_pool.clone();
        tasks.spawn(async move {
            create_attempt(
                &pool,
                CreateCollectionAttempt {
                    tenant_id: tenant,
                    invoice_id: invoice,
                    provider_code: "wompi",
                    idempotency_key: "collect-0001",
                    amount_minor: 50000,
                    currency: "COP",
                },
            )
            .await
        });
    }
    let mut attempt_id = None;
    let mut created = 0;
    while let Some(result) = tasks.join_next().await {
        let attempt = result
            .expect("collection task joins")
            .expect("collection attempt is idempotent");
        if !attempt.replayed {
            created += 1;
        }
        if let Some(expected) = attempt_id {
            assert_eq!(attempt.id, expected);
        } else {
            attempt_id = Some(attempt.id);
        }
    }
    assert_eq!(created, 1);
    let attempt = attempt_id.expect("one SaaS collection attempt exists");
    assert!(
        !transition_attempt(
            &db.runtime_pool,
            tenant,
            attempt,
            "submitted",
            Some("wompi-ref-1")
        )
        .await
        .expect("submission transition")
    );
    assert!(
        !transition_attempt(&db.runtime_pool, tenant, attempt, "processing", None)
            .await
            .expect("processing transition")
    );
    assert!(
        !transition_attempt(&db.runtime_pool, tenant, attempt, "succeeded", None)
            .await
            .expect("success transition")
    );
    assert!(
        !transition_attempt(&db.runtime_pool, tenant, attempt, "succeeded", None)
            .await
            .expect("terminal replay is idempotent")
    );
    assert!(
        transition_attempt(&db.runtime_pool, tenant, attempt, "failed", None)
            .await
            .is_err(),
        "terminal collection state cannot be reversed"
    );
    let retry = create_attempt(
        &db.runtime_pool,
        CreateCollectionAttempt {
            tenant_id: tenant,
            invoice_id: invoice,
            provider_code: "wompi",
            idempotency_key: "collect-0002",
            amount_minor: 50000,
            currency: "COP",
        },
    )
    .await;
    assert!(
        retry.is_err(),
        "new commercial retry key stays blocked until policy approval"
    );
    let collection_journal:i64=sqlx::query_scalar("SELECT count(*) FROM platform_billing.ledger_journals WHERE tenant_id=$1 AND journal_key=$2")
        .bind(tenant).bind(format!("collection-succeeded:{attempt}")).fetch_one(&db.migration_pool).await.expect("collection journal is queryable");
    assert_eq!(
        collection_journal, 1,
        "successful collection posts one balanced accounting journal"
    );
    let buyer_payments: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payments.payment_intents WHERE tenant_id=$1")
            .bind(tenant)
            .fetch_one(&db.migration_pool)
            .await
            .expect("buyer payment table is readable");
    assert_eq!(
        buyer_payments, 0,
        "SaaS collection never creates a tenant buyer payment intent"
    );

    let event_id = Uuid::new_v4();
    let payload = json!({"external_document_id":"fiscal-123","status":"accepted"});
    let first = receive(
        &db.runtime_pool,
        tenant,
        "fiscal-service",
        event_id,
        "fiscal_document.status_changed",
        1,
        &payload,
    )
    .await
    .expect("first fiscal response is durably received");
    let duplicate = receive(
        &db.runtime_pool,
        tenant,
        "fiscal-service",
        event_id,
        "fiscal_document.status_changed",
        1,
        &payload,
    )
    .await
    .expect("duplicate delivery is recognized");
    assert!(matches!(first, InboxDisposition::Inserted(_)));
    assert!(matches!(duplicate, InboxDisposition::Duplicate(_)));
    let conflicting = receive(
        &db.runtime_pool,
        tenant,
        "fiscal-service",
        event_id,
        "fiscal_document.status_changed",
        1,
        &json!({"status":"rejected"}),
    )
    .await;
    assert!(
        conflicting.is_err(),
        "same event ID with a different payload is rejected"
    );

    let claimed = outbox::claim(&db.runtime_pool, 16)
        .await
        .expect("billing outbox claims pending event");
    assert_eq!(claimed.len(), 5);
    let old = claimed[0].clone();
    sqlx::query("UPDATE platform_billing.outbox_events SET lease_until=now()-interval '1 second' WHERE event_id=$1")
        .bind(old.event_id).execute(&db.migration_pool).await.expect("simulate worker crash after lease");
    let recovered = outbox::claim(&db.runtime_pool, 16)
        .await
        .expect("expired lease is recovered");
    assert_eq!(recovered.len(), 1);
    assert!(
        !outbox::acknowledge(&db.runtime_pool, old.event_id, old.lease_token)
            .await
            .expect("stale lease ack is evaluated")
    );
    assert!(
        outbox::acknowledge(
            &db.runtime_pool,
            recovered[0].event_id,
            recovered[0].lease_token
        )
        .await
        .expect("current lease acknowledges")
    );

    let other = Uuid::new_v4();
    let mut tx = db
        .runtime_pool
        .begin()
        .await
        .expect("tenant read transaction begins");
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(other.to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant scope is set");
    let visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM platform_billing.invoices WHERE id=$1")
            .bind(invoice)
            .fetch_one(&mut *tx)
            .await
            .expect("RLS filters cross-tenant invoice");
    assert_eq!(visible, 0);
    tx.rollback().await.expect("read transaction rolls back");

    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unbalanced_financial_journal_rolls_back_as_one_transaction() {
    let db = TestDatabase::new().await;
    let service = BillingService::new(db.runtime_pool.clone());
    let tenant = Uuid::new_v4();
    let now = Utc::now();
    let invoice = service
        .create_draft(CreateInvoice {
            tenant_id: tenant,
            invoice_number: "PV-ROLLBACK-01",
            currency: "COP",
            period_start: now - Duration::days(1),
            period_end: now,
            due_at: None,
            correlation_id: "rollback",
        })
        .await
        .expect("draft invoice is persisted");
    let journal = Uuid::new_v4();
    let mut tx = db
        .migration_pool
        .begin()
        .await
        .expect("ledger transaction begins");
    sqlx::query("INSERT INTO platform_billing.ledger_journals (id,tenant_id,invoice_id,journal_key,currency) VALUES ($1,$2,$3,$4,'COP')")
        .bind(journal).bind(tenant).bind(invoice).bind(format!("rollback:{journal}"))
        .execute(&mut *tx).await.expect("journal header is staged");
    sqlx::query("INSERT INTO platform_billing.ledger_entries (id,tenant_id,journal_id,entry_number,account_code,direction,amount_minor,currency) VALUES ($1,$2,$3,1,'accounts_receivable','debit',100,'COP')")
        .bind(Uuid::new_v4()).bind(tenant).bind(journal)
        .execute(&mut *tx).await.expect("unbalanced entry is staged before deferred validation");
    assert!(
        tx.commit().await.is_err(),
        "deferred balance constraint rejects incomplete journal"
    );
    let journals: i64 =
        sqlx::query_scalar("SELECT count(*) FROM platform_billing.ledger_journals WHERE id=$1")
            .bind(journal)
            .fetch_one(&db.migration_pool)
            .await
            .expect("rollback state is readable");
    let entries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_billing.ledger_entries WHERE journal_id=$1",
    )
    .bind(journal)
    .fetch_one(&db.migration_pool)
    .await
    .expect("rollback entries are readable");
    assert_eq!(
        journals, 0,
        "journal header rolls back with its invalid entry"
    );
    assert_eq!(entries, 0, "unbalanced ledger entry rolls back");
    db.finish().await;
}
