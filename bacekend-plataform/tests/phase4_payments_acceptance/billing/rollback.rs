use chrono::{Duration, Utc};
use platform_billing::application::{BillingService, CreateInvoice};
use uuid::Uuid;

use super::super::database::TestDatabase;

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
