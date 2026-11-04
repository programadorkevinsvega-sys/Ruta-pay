use platform_payments::application::payment_service::{PaymentService, PaymentServiceError};
use sqlx::Row;
use uuid::Uuid;

use super::super::database::TestDatabase;
use super::support::{attempt_command, intent_command, status_command};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_transitions_are_guarded_atomic_and_create_durable_reconciliation() {
    let database = TestDatabase::new().await;
    let service = PaymentService::new(database.runtime_pool.clone());
    let tenant_id = Uuid::new_v4();
    let actor_key_id = Uuid::new_v4();
    let (intent, _) = service
        .create_payment_intent(intent_command(tenant_id, actor_key_id))
        .await
        .expect("intent is created");
    let (attempt, replayed) = service
        .create_payment_attempt(attempt_command(
            tenant_id,
            actor_key_id,
            intent.id,
            "attempt-status-001",
        ))
        .await
        .expect("attempt is created");
    assert!(!replayed);

    let submitted = status_command(
        tenant_id,
        actor_key_id,
        intent.id,
        attempt.id,
        "processing",
        1,
    );
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let service = service.clone();
        let command = submitted.clone();
        tasks.spawn(async move { service.update_payment_intent_status(command).await });
    }
    while let Some(result) = tasks.join_next().await {
        let update = result
            .expect("transition task completes")
            .expect("concurrent duplicate transition is idempotent");
        assert_eq!(update.intent.status, "processing");
        assert_eq!(update.aggregate_version, 2);
    }

    let (replay, replayed) = service
        .create_payment_attempt(attempt_command(
            tenant_id,
            actor_key_id,
            intent.id,
            "attempt-status-001",
        ))
        .await
        .expect("same idempotency key can replay after attempt submission");
    assert!(replayed);
    assert_eq!(replay.id, attempt.id);
    assert_eq!(replay.status, "submitted");

    let transaction_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payments.payment_transactions (id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_transaction_reference,amount_minor,currency,status,aggregate_version) VALUES ($1,$2,'test',$3,$4,'wompi','account_01','provider-tx-accepted',1299,'COP','pending',1)",
    )
    .bind(transaction_id)
    .bind(tenant_id)
    .bind(intent.id)
    .bind(attempt.id)
    .execute(&database.migration_pool)
    .await
    .expect("submitted attempt may own a matching provider transaction");

    let transactions = service
        .list_payment_transactions(
            tenant_id,
            "test",
            intent.id,
            actor_key_id,
            "phase4-transactions-list",
            50,
        )
        .await
        .expect("tenant can query transactions");
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].id, transaction_id);
    assert_eq!(transactions[0].amount_minor, intent.amount_minor);
    assert_eq!(transactions[0].currency, intent.currency);

    let foreign = service
        .list_payment_transactions(
            Uuid::new_v4(),
            "test",
            intent.id,
            Uuid::new_v4(),
            "phase4-transactions-cross-tenant",
            50,
        )
        .await;
    assert!(matches!(
        foreign,
        Err(PaymentServiceError::Domain(
            platform_payments::domain::payment_intent::PaymentIntentError::NotFound
        ))
    ));

    let unknown = service
        .update_payment_intent_status(status_command(
            tenant_id,
            actor_key_id,
            intent.id,
            attempt.id,
            "outcome_unknown",
            2,
        ))
        .await
        .expect("timeout moves intent and attempt to uncertain state");
    assert_eq!(unknown.intent.status, "outcome_unknown");
    assert_eq!(unknown.aggregate_version, 3);

    assert!(matches!(
        service
            .update_payment_intent_status(status_command(
                tenant_id,
                actor_key_id,
                intent.id,
                attempt.id,
                "succeeded",
                3,
            ))
            .await,
        Err(PaymentServiceError::StatusUpdateNotAllowed)
    ));

    let case = sqlx::query(
        "SELECT id,payment_transaction_id,status FROM payments.payment_reconciliation_cases WHERE payment_intent_id=$1 AND payment_attempt_id=$2",
    )
    .bind(intent.id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("outcome_unknown creates one durable case");
    let case_id: Uuid = Row::try_get(&case, "id").expect("case id is present");
    let linked_transaction: Option<Uuid> =
        Row::try_get(&case, "payment_transaction_id").expect("transaction link is present");
    let case_status: String = Row::try_get(&case, "status").expect("case status is present");
    assert_eq!(linked_transaction, Some(transaction_id));
    assert_eq!(case_status, "pending");

    let statuses = sqlx::query(
        "SELECT i.status AS intent_status,a.status AS attempt_status FROM payments.payment_intents i JOIN payments.payment_attempts a ON a.payment_intent_id=i.id WHERE i.id=$1 AND a.id=$2",
    )
    .bind(intent.id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("aggregate states are queryable");
    assert_eq!(
        Row::try_get::<String, _>(&statuses, "intent_status").expect("intent status"),
        "outcome_unknown"
    );
    assert_eq!(
        Row::try_get::<String, _>(&statuses, "attempt_status").expect("attempt status"),
        "outcome_unknown"
    );

    let intent_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.audit_events WHERE resource_id=$1 AND action='payment_intent.status_changed'",
    )
    .bind(intent.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("intent transition audit is queryable");
    assert_eq!(intent_audit_count, 2);

    let attempt_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.audit_events WHERE resource_id=$1 AND action='payment_attempt.status_changed'",
    )
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("attempt transition audit is queryable");
    assert_eq!(
        attempt_audit_count, 2,
        "attempt audit rows identify the attempt resource"
    );

    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.outbox_events WHERE aggregate_id IN ($1,$2,$3)",
    )
    .bind(intent.id)
    .bind(attempt.id)
    .bind(case_id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("all lifecycle events are durable");
    assert_eq!(events, 7);
    database.finish().await;
}
