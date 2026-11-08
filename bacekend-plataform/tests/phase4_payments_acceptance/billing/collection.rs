use chrono::{Duration, Utc};
use platform_billing::{
    application::{BillingService, CreateInvoice, InvoiceLine},
    infrastructure::{
        collection::{CreateCollectionAttempt, create_attempt, transition_attempt},
        inbox::{InboxDisposition, receive},
        outbox,
    },
};
use serde_json::json;
use uuid::Uuid;

use super::super::database::TestDatabase;

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
    assert!(matches!(duplicate, InboxDisposition::Duplicate { .. }));
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
