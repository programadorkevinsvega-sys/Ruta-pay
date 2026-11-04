use platform_payments::application::payment_service::{PaymentService, PaymentServiceError};
use uuid::Uuid;

use super::super::database::TestDatabase;
use super::support::{attempt_command, intent_command};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attempt_creation_is_idempotent_under_concurrency_and_disallows_unapproved_retries() {
    let database = TestDatabase::new().await;
    let service = PaymentService::new(database.runtime_pool.clone());
    let tenant_id = Uuid::new_v4();
    let actor_key_id = Uuid::new_v4();
    let (intent, replayed) = service
        .create_payment_intent(intent_command(tenant_id, actor_key_id))
        .await
        .expect("payment intent is created");
    assert!(!replayed);

    let command = attempt_command(tenant_id, actor_key_id, intent.id, "attempt-create-001");
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let service = service.clone();
        let command = command.clone();
        tasks.spawn(async move { service.create_payment_attempt(command).await });
    }

    let mut attempt_id = None;
    let mut first_creations = 0;
    while let Some(result) = tasks.join_next().await {
        let (attempt, replayed) = result
            .expect("concurrent task completes")
            .expect("concurrent idempotent creation succeeds");
        if !replayed {
            first_creations += 1;
        }
        assert_eq!(attempt.attempt_number, 1);
        if let Some(expected) = attempt_id {
            assert_eq!(attempt.id, expected);
        } else {
            attempt_id = Some(attempt.id);
        }
    }
    assert_eq!(first_creations, 1);
    let attempt_id = attempt_id.expect("one shared attempt is returned");

    let mut changed = command.clone();
    changed.provider_account_reference = "different_account".to_owned();
    assert!(matches!(
        service.create_payment_attempt(changed).await,
        Err(PaymentServiceError::Domain(
            platform_payments::domain::payment_intent::PaymentIntentError::IdempotencyConflict
        ))
    ));

    assert!(matches!(
        service
            .create_payment_attempt(attempt_command(
                tenant_id, actor_key_id, intent.id, "attempt-create-002",
            ))
            .await,
        Err(PaymentServiceError::AttemptAlreadyExists)
    ));

    let attempts = service
        .list_payment_attempts(
            tenant_id, "test", intent.id, actor_key_id, "phase4-attempt-list", 50,
        )
        .await
        .expect("tenant can list its attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].id, attempt_id);
    assert_eq!(attempts[0].status, "created");

    let foreign = service
        .list_payment_attempts(
            Uuid::new_v4(), "test", intent.id, Uuid::new_v4(), "phase4-attempt-cross-tenant", 50,
        )
        .await;
    assert!(matches!(
        foreign,
        Err(PaymentServiceError::Domain(
            platform_payments::domain::payment_intent::PaymentIntentError::NotFound
        ))
    ));

    let outbox: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payments.outbox_events WHERE aggregate_id=$1")
            .bind(attempt_id)
            .fetch_one(&database.migration_pool)
            .await
            .expect("attempt event is persisted once");
    assert_eq!(outbox, 1);
    database.finish().await;
}
