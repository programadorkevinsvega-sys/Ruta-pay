use super::database::TestDatabase;
use platform_payments::application::payment_service::{CreatePaymentIntentCommand, PaymentService};
use sqlx::Row;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lifecycle_storage_enforces_ownership_deduplication_and_reconciliation_integrity() {
    let database = TestDatabase::new().await;
    let tenant_id = Uuid::new_v4();
    let actor_key_id = Uuid::new_v4();
    let service = PaymentService::new(database.runtime_pool.clone());
    let (intent, replayed) = service
        .create_payment_intent(CreatePaymentIntentCommand {
            tenant_id,
            actor_key_id,
            environment: "test".to_owned(),
            amount_minor: 1299,
            currency: "COP".to_owned(),
            payment_method_type: "card".to_owned(),
            merchant_reference: Some("lifecycle-storage".to_owned()),
            idempotency_key: "lifecycle-storage-001".to_owned(),
            correlation_id: "lifecycle-storage-acceptance".to_owned(),
        })
        .await
        .expect("parent payment intent is persisted");
    assert!(!replayed);

    let attempt_id = Uuid::new_v4();
    let transaction_id = Uuid::new_v4();
    let inbox_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let delivery_id = Uuid::new_v4();
    let reconciliation_id = Uuid::new_v4();
    let reconciliation_attempt_id = Uuid::new_v4();

    let mut transaction = database
        .runtime_pool
        .begin()
        .await
        .expect("tenant transaction starts");
    set_tenant(&mut transaction, tenant_id).await;
    sqlx::query(
        "INSERT INTO payments.payment_attempts (id,tenant_id,environment,payment_intent_id,attempt_number,provider_code,provider_account_reference,status,aggregate_version,last_event_at) VALUES ($1,$2,'test',$3,1,'wompi','account_01','submitted',2,now())",
    )
    .bind(attempt_id)
    .bind(tenant_id)
    .bind(intent.id)
    .execute(&mut *transaction)
    .await
    .expect("submitted attempt is linked to its tenant and intent");

    sqlx::query(
        "INSERT INTO payments.payment_transactions (id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_transaction_reference,amount_minor,currency,status,aggregate_version) VALUES ($1,$2,'test',$3,$4,'wompi','account_01','provider-tx-01',1299,'COP','pending',1)",
    )
    .bind(transaction_id)
    .bind(tenant_id)
    .bind(intent.id)
    .bind(attempt_id)
    .execute(&mut *transaction)
    .await
    .expect("transaction inherits intent money and attempt provider ownership");

    sqlx::query(
        "INSERT INTO payments.provider_event_inbox (inbox_id,event_id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_event_id,provider_transaction_reference,normalized_status,schema_version,occurred_at,payload_sha256,correlation_id,producer_identity) VALUES ($1,$2,$3,'test',$4,$5,'wompi','account_01','provider-event-01','provider-tx-01','processing',1,now(),$6,'lifecycle-storage-acceptance','spiffe://gateway/payments')",
    )
    .bind(inbox_id)
    .bind(event_id)
    .bind(tenant_id)
    .bind(intent.id)
    .bind(attempt_id)
    .bind(vec![7_u8; 32])
    .execute(&mut *transaction)
    .await
    .expect("normalized event envelope is stored without raw payload");

    sqlx::query(
        "INSERT INTO payments.provider_event_deliveries (delivery_id,tenant_id,environment,inbox_id,event_id,provider_code,provider_account_reference,provider_event_id,producer_identity,disposition) VALUES ($1,$2,'test',$3,$4,'wompi','account_01','provider-event-01','spiffe://gateway/payments','accepted')",
    )
    .bind(delivery_id)
    .bind(tenant_id)
    .bind(inbox_id)
    .bind(event_id)
    .execute(&mut *transaction)
    .await
    .expect("accepted delivery is associated with its canonical inbox event");

    sqlx::query(
        "INSERT INTO payments.provider_event_deliveries (delivery_id,tenant_id,environment,producer_identity,disposition,rejection_code) VALUES ($1,$2,'test','spiffe://gateway/payments','rejected','tenant_scope_mismatch')",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(&mut *transaction)
    .await
    .expect("rejected authenticated delivery is retained without trusting an inbox event");

    sqlx::query(
        "INSERT INTO payments.payment_reconciliation_cases (id,tenant_id,environment,payment_intent_id,payment_attempt_id,payment_transaction_id,reason,status,attempts) VALUES ($1,$2,'test',$3,$4,$5,'outcome_unknown','pending',0)",
    )
    .bind(reconciliation_id)
    .bind(tenant_id)
    .bind(intent.id)
    .bind(attempt_id)
    .bind(transaction_id)
    .execute(&mut *transaction)
    .await
    .expect("durable reconciliation case is scoped to the original attempt");

    sqlx::query(
        "INSERT INTO payments.payment_reconciliation_attempts (id,tenant_id,environment,reconciliation_case_id,attempt_number,result_status,evidence_sha256,safe_error_code) VALUES ($1,$2,'test',$3,1,'inconclusive',$4,'provider_result_not_authoritative')",
    )
    .bind(reconciliation_attempt_id)
    .bind(tenant_id)
    .bind(reconciliation_id)
    .bind(vec![9_u8; 32])
    .execute(&mut *transaction)
    .await
    .expect("reconciliation query evidence is durable and redacted");
    transaction
        .commit()
        .await
        .expect("lifecycle records commit atomically");

    let mut same_tenant = database
        .runtime_pool
        .begin()
        .await
        .expect("read transaction starts");
    set_tenant(&mut same_tenant, tenant_id).await;
    let counts = sqlx::query(
        "SELECT (SELECT count(*) FROM payments.payment_attempts WHERE id=$1) AS attempts,(SELECT count(*) FROM payments.payment_transactions WHERE id=$2) AS transactions,(SELECT count(*) FROM payments.provider_event_inbox WHERE inbox_id=$3) AS inbox,(SELECT count(*) FROM payments.provider_event_deliveries WHERE inbox_id=$3) AS deliveries,(SELECT count(*) FROM payments.payment_reconciliation_cases WHERE id=$4) AS cases,(SELECT count(*) FROM payments.payment_reconciliation_attempts WHERE id=$5) AS reconciliation_attempts",
    )
    .bind(attempt_id)
    .bind(transaction_id)
    .bind(inbox_id)
    .bind(reconciliation_id)
    .bind(reconciliation_attempt_id)
    .fetch_one(&mut *same_tenant)
    .await
    .expect("tenant can read its own lifecycle records");
    for column in [
        "attempts",
        "transactions",
        "inbox",
        "deliveries",
        "cases",
        "reconciliation_attempts",
    ] {
        let expected = if column == "deliveries" { 2 } else { 1 };
        assert_eq!(
            counts.get::<i64, _>(column),
            expected,
            "{column} rows are visible"
        );
    }
    same_tenant
        .commit()
        .await
        .expect("read transaction commits");

    let other_tenant = Uuid::new_v4();
    let mut isolated = database
        .runtime_pool
        .begin()
        .await
        .expect("isolated transaction starts");
    set_tenant(&mut isolated, other_tenant).await;
    let visible: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM payments.payment_attempts WHERE id=$1) + (SELECT count(*) FROM payments.payment_transactions WHERE id=$2) + (SELECT count(*) FROM payments.provider_event_inbox WHERE inbox_id=$3) + (SELECT count(*) FROM payments.provider_event_deliveries WHERE delivery_id=$4) + (SELECT count(*) FROM payments.payment_reconciliation_cases WHERE id=$5) + (SELECT count(*) FROM payments.payment_reconciliation_attempts WHERE id=$6)",
    )
    .bind(attempt_id)
    .bind(transaction_id)
    .bind(inbox_id)
    .bind(delivery_id)
    .bind(reconciliation_id)
    .bind(reconciliation_attempt_id)
    .fetch_one(&mut *isolated)
    .await
    .expect("cross-tenant lookups are evaluated under each table's RLS policy");
    assert_eq!(
        visible, 0,
        "known foreign identifiers do not bypass lifecycle table RLS"
    );
    let changed = sqlx::query("UPDATE payments.payment_attempts SET status='failed' WHERE id=$1")
        .bind(attempt_id)
        .execute(&mut *isolated)
        .await
        .expect("cross-tenant update is filtered by RLS");
    assert_eq!(changed.rows_affected(), 0);
    isolated
        .commit()
        .await
        .expect("isolated transaction commits");

    let mut duplicate_event = database
        .runtime_pool
        .begin()
        .await
        .expect("duplicate test starts");
    set_tenant(&mut duplicate_event, tenant_id).await;
    let duplicate = sqlx::query(
        "INSERT INTO payments.provider_event_inbox (inbox_id,event_id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_event_id,normalized_status,schema_version,occurred_at,payload_sha256,correlation_id,producer_identity) VALUES ($1,$2,$3,'test',$4,$5,'wompi','account_01','provider-event-01','succeeded',1,now(),$6,'duplicate-delivery','spiffe://gateway/payments')",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(intent.id)
    .bind(attempt_id)
    .bind(vec![8_u8; 32])
    .execute(&mut *duplicate_event)
    .await;
    assert!(
        duplicate.is_err(),
        "provider/account/event identity is unique"
    );
    duplicate_event
        .rollback()
        .await
        .expect("duplicate transaction rolls back");

    let mut duplicate_case = database
        .runtime_pool
        .begin()
        .await
        .expect("case uniqueness test starts");
    set_tenant(&mut duplicate_case, tenant_id).await;
    let duplicate = sqlx::query(
        "INSERT INTO payments.payment_reconciliation_cases (id,tenant_id,environment,payment_intent_id,payment_attempt_id,reason,status) VALUES ($1,$2,'test',$3,$4,'outcome_unknown','pending')",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(intent.id)
    .bind(attempt_id)
    .execute(&mut *duplicate_case)
    .await;
    assert!(
        duplicate.is_err(),
        "an attempt has at most one unresolved reconciliation case"
    );
    duplicate_case
        .rollback()
        .await
        .expect("duplicate case rolls back");

    let created_attempt_id = Uuid::new_v4();
    let mut created_attempt = database
        .runtime_pool
        .begin()
        .await
        .expect("created attempt transaction starts");
    set_tenant(&mut created_attempt, tenant_id).await;
    sqlx::query(
        "INSERT INTO payments.payment_attempts (id,tenant_id,environment,payment_intent_id,attempt_number,provider_code,provider_account_reference,status) VALUES ($1,$2,'test',$3,2,'wompi','account_01','created')",
    )
    .bind(created_attempt_id)
    .bind(tenant_id)
    .bind(intent.id)
    .execute(&mut *created_attempt)
    .await
    .expect("a second historical attempt can be prepared without executing a charge");
    created_attempt
        .commit()
        .await
        .expect("prepared attempt commits");

    let mut premature_transaction = database
        .runtime_pool
        .begin()
        .await
        .expect("transaction guard test starts");
    set_tenant(&mut premature_transaction, tenant_id).await;
    let rejected = sqlx::query(
        "INSERT INTO payments.payment_transactions (id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_transaction_reference,amount_minor,currency,status) VALUES ($1,$2,'test',$3,$4,'wompi','account_01','provider-tx-before-submit',1299,'COP','pending')",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(intent.id)
    .bind(created_attempt_id)
    .execute(&mut *premature_transaction)
    .await;
    assert!(
        rejected.is_err(),
        "transaction cannot predate operation submission"
    );
    premature_transaction
        .rollback()
        .await
        .expect("premature transaction rolls back");

    let mut rollback = database
        .runtime_pool
        .begin()
        .await
        .expect("rollback transaction starts");
    set_tenant(&mut rollback, tenant_id).await;
    let rolled_back_attempt_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payments.payment_attempts (id,tenant_id,environment,payment_intent_id,attempt_number,provider_code,provider_account_reference,status) VALUES ($1,$2,'test',$3,3,'wompi','account_01','created')",
    )
    .bind(rolled_back_attempt_id)
    .bind(tenant_id)
    .bind(intent.id)
    .execute(&mut *rollback)
    .await
    .expect("temporary lifecycle write succeeds");
    rollback
        .rollback()
        .await
        .expect("temporary lifecycle write rolls back");

    let mut verify_rollback = database
        .runtime_pool
        .begin()
        .await
        .expect("rollback verification starts");
    set_tenant(&mut verify_rollback, tenant_id).await;
    let persisted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payments.payment_attempts WHERE id=$1")
            .bind(rolled_back_attempt_id)
            .fetch_one(&mut *verify_rollback)
            .await
            .expect("rolled-back attempt can be checked");
    assert_eq!(persisted, 0, "rollback leaves no partial lifecycle record");
    verify_rollback
        .commit()
        .await
        .expect("rollback verification commits");

    database.finish().await;
}

async fn set_tenant(transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>, tenant_id: Uuid) {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id.to_string())
        .execute(&mut **transaction)
        .await
        .expect("tenant context is transaction-local");
}
