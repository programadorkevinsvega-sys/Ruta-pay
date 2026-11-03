#[path = "phase4_payments_acceptance/database.rs"]
mod database;

use platform_payments::{
    application::payment_service::{CreatePaymentIntentCommand, PaymentService},
    domain::payment_intent::PaymentIntentError,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn payment_intent_creation_replay_conflict_concurrency_and_tenant_isolation() {
    let database = database::TestDatabase::new().await;
    let service = PaymentService::new(database.runtime_pool.clone());
    let tenant_id = Uuid::new_v4();
    let other_tenant_id = Uuid::new_v4();
    let key_id = Uuid::new_v4();
    let original = command(tenant_id, key_id, "phase4-create-001");

    let (created, replayed) = service
        .create_payment_intent(original.clone())
        .await
        .expect("first intent creation commits");
    assert!(!replayed);
    assert_eq!(created.status, "created");
    assert_eq!(created.amount_minor, 1299);
    assert_eq!(created.currency, "COP");

    let (replay, replayed) = service
        .create_payment_intent(original.clone())
        .await
        .expect("same request replays its original response snapshot");
    assert!(replayed);
    assert_eq!(replay, created);

    let mut changed = original.clone();
    changed.amount_minor += 1;
    assert!(matches!(
        service.create_payment_intent(changed).await,
        Err(
            platform_payments::application::payment_service::PaymentServiceError::Domain(
                PaymentIntentError::IdempotencyConflict
            )
        )
    ));

    let mut tasks = tokio::task::JoinSet::new();
    let concurrent = command(tenant_id, key_id, "phase4-concurrent-001");
    for _ in 0..12 {
        let service = service.clone();
        let concurrent = concurrent.clone();
        tasks.spawn(async move { service.create_payment_intent(concurrent).await });
    }
    let mut concurrent_id = None;
    let mut initial_results = 0;
    while let Some(result) = tasks.join_next().await {
        let (intent, replayed) = result
            .expect("concurrent caller task completes")
            .expect("concurrent idempotent request completes");
        if !replayed {
            initial_results += 1;
        }
        if let Some(expected) = concurrent_id {
            assert_eq!(intent.id, expected, "all callers receive one intent");
        } else {
            concurrent_id = Some(intent.id);
        }
    }
    assert_eq!(initial_results, 1, "one request owns the idempotency key");

    let fetched = service
.get_payment_intent(tenant_id, "test", created.id, key_id, "phase4-read-001")
        .await
        .expect("a tenant API key reads its own payment intent in the same environment");
    assert_eq!(fetched, created);

    assert!(matches!(
        service
            .get_payment_intent(
                other_tenant_id,
                "test",
                created.id,
                key_id,
                "phase4-cross-tenant"
            )
            .await,
        Err(
            platform_payments::application::payment_service::PaymentServiceError::Domain(
                PaymentIntentError::NotFound
            )
        )
    ));

    assert!(matches!(
        service
            .get_payment_intent(
                tenant_id,
                "live",
                created.id,
                key_id,
                "phase4-cross-environment"
            )
            .await,
        Err(
            platform_payments::application::payment_service::PaymentServiceError::Domain(
                PaymentIntentError::NotFound
            )
        )
    ));

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.outbox_events WHERE aggregate_id IN ($1,$2)",
    )
    .bind(created.id)
    .bind(concurrent_id.expect("concurrent payment intent id exists"))
    .fetch_one(&database.migration_pool)
    .await
    .expect("outbox rows can be inspected by the migration role");
    assert_eq!(
        outbox_count, 2,
        "each intent emits exactly one durable event"
    );

    let audit_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payments.audit_events WHERE resource_id=$1")
            .bind(created.id)
            .fetch_one(&database.migration_pool)
            .await
            .expect("audit rows can be inspected by the migration role");
    assert_eq!(audit_count, 3, "creation, replay and read are audited");

    database.finish().await;
}

fn command(
    tenant_id: Uuid,
    actor_key_id: Uuid,
    idempotency_key: &str,
) -> CreatePaymentIntentCommand {
    CreatePaymentIntentCommand {
        tenant_id,
        actor_key_id,
        environment: "test".to_owned(),
        amount_minor: 1299,
        currency: "COP".to_owned(),
        payment_method_type: "card".to_owned(),
        merchant_reference: Some("merchant-order-01".to_owned()),
        idempotency_key: idempotency_key.to_owned(),
        correlation_id: "phase4_payments_acceptance".to_owned(),
    }
}
