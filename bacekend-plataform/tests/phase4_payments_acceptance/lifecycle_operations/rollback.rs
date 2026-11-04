use platform_payments::application::payment_service::{PaymentService, PaymentServiceError};
use sqlx::Row;
use uuid::Uuid;

use super::super::database::TestDatabase;
use super::support::{attempt_command, intent_command, status_command};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_outbox_write_rolls_back_attempt_intent_and_audit_changes() {
    let database = TestDatabase::new().await;
    let service = PaymentService::new(database.runtime_pool.clone());
    let tenant_id = Uuid::new_v4();
    let actor_key_id = Uuid::new_v4();
    let (intent, _) = service
        .create_payment_intent(intent_command(tenant_id, actor_key_id))
        .await
        .expect("intent is created");
    let (attempt, _) = service
        .create_payment_attempt(attempt_command(
            tenant_id,
            actor_key_id,
            intent.id,
            "attempt-rollback-001",
        ))
        .await
        .expect("attempt is created");

    sqlx::raw_sql(
        "CREATE FUNCTION payments.fail_status_outbox_acceptance() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_type = 'payment_intent.status_changed' THEN RAISE EXCEPTION 'injected outbox write failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER fail_status_outbox_acceptance BEFORE INSERT ON payments.outbox_events FOR EACH ROW EXECUTE FUNCTION payments.fail_status_outbox_acceptance();",
    )
    .execute(&database.migration_pool)
    .await
    .expect("isolated database receives an outbox failure trigger");

    let result = service
        .update_payment_intent_status(status_command(
            tenant_id,
            actor_key_id,
            intent.id,
            attempt.id,
            "processing",
            1,
        ))
        .await;
    assert!(matches!(result, Err(PaymentServiceError::Database(_))));

    let states = sqlx::query(
        "SELECT i.status AS intent_status,i.aggregate_version,a.status AS attempt_status,a.aggregate_version AS attempt_version FROM payments.payment_intents i JOIN payments.payment_attempts a ON a.payment_intent_id=i.id WHERE i.id=$1 AND a.id=$2",
    )
    .bind(intent.id)
    .bind(attempt.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("rolled back aggregates remain queryable");
    assert_eq!(
        Row::try_get::<String, _>(&states, "intent_status").expect("intent status"),
        "created"
    );
    assert_eq!(
        Row::try_get::<String, _>(&states, "attempt_status").expect("attempt status"),
        "created"
    );
    assert_eq!(
        Row::try_get::<i64, _>(&states, "aggregate_version").expect("intent version"),
        1
    );
    assert_eq!(
        Row::try_get::<i64, _>(&states, "attempt_version").expect("attempt version"),
        1
    );

    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.audit_events WHERE resource_id=$1 AND action='payment_intent.status_changed'",
    )
    .bind(intent.id)
    .fetch_one(&database.migration_pool)
    .await
    .expect("audit rows are queryable");
    assert_eq!(audit, 0);

    sqlx::raw_sql(
        "DROP TRIGGER fail_status_outbox_acceptance ON payments.outbox_events; DROP FUNCTION payments.fail_status_outbox_acceptance();",
    )
    .execute(&database.migration_pool)
    .await
    .expect("isolated failure trigger is removed");
    database.finish().await;
}
