use platform_payments::application::payment_service::{
    CreatePaymentAttemptCommand, CreatePaymentIntentCommand, PaymentService,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::database::TestDatabase;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_provider_event_deliveries_create_one_canonical_inbox_row() {
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
            merchant_reference: Some("inbox-concurrency".to_owned()),
            idempotency_key: format!("intent-{}", Uuid::new_v4()),
            correlation_id: "phase4-inbox-concurrency".to_owned(),
        })
        .await
        .expect("parent intent is created");
    assert!(!replayed);

    let (attempt, attempt_replayed) = service
        .create_payment_attempt(CreatePaymentAttemptCommand {
            tenant_id,
            actor_key_id,
            environment: "test".to_owned(),
            payment_intent_id: intent.id,
            provider_code: "wompi".to_owned(),
            provider_account_reference: "account_01".to_owned(),
            idempotency_key: "inbox-attempt-001".to_owned(),
            correlation_id: "phase4-inbox-concurrency".to_owned(),
        })
        .await
        .expect("parent attempt is created");
    assert!(!attempt_replayed);

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..2 {
        let pool = database.runtime_pool.clone();
        let inbox_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let payload_hash = vec![5_u8; 32];
        tasks.spawn(async move {
            insert_inbox_delivery(
                &pool,
                tenant_id,
                intent.id,
                attempt.id,
                inbox_id,
                event_id,
                payload_hash,
            )
            .await
        });
    }

    let mut inserted = 0;
    let mut duplicated = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("concurrent insert task completes") {
            Ok(()) => inserted += 1,
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23505") => {
                duplicated += 1;
            }
            Err(error) => panic!("unexpected inbox insert error: {error}"),
        }
    }
    assert_eq!(
        inserted, 1,
        "only one delivery creates the canonical inbox row"
    );
    assert_eq!(
        duplicated, 1,
        "the unique key rejects the concurrent duplicate"
    );

    let mut check = database
        .runtime_pool
        .begin()
        .await
        .expect("read transaction starts");
    set_tenant(&mut check, tenant_id)
        .await
        .expect("tenant context is set for the scoped inbox read");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.provider_event_inbox WHERE tenant_id=$1 AND environment='test' AND provider_code='wompi' AND provider_account_reference='account_01' AND provider_event_id='concurrent-provider-event-001'",
    )
    .bind(tenant_id)
    .fetch_one(&mut *check)
    .await
    .expect("tenant can read its canonical inbox event");
    assert_eq!(count, 1);
    check.commit().await.expect("read transaction commits");
    database.finish().await;
}

async fn insert_inbox_delivery(
    pool: &PgPool,
    tenant_id: Uuid,
    payment_intent_id: Uuid,
    payment_attempt_id: Uuid,
    inbox_id: Uuid,
    event_id: Uuid,
    payload_hash: Vec<u8>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    sqlx::query(
        "INSERT INTO payments.provider_event_inbox (inbox_id,event_id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_event_id,normalized_status,schema_version,occurred_at,payload_sha256,correlation_id,producer_identity) VALUES ($1,$2,$3,'test',$4,$5,'wompi','account_01','concurrent-provider-event-001','processing',1,now(),$6,'phase4-inbox-concurrency','spiffe://gateway/payments')",
    )
    .bind(inbox_id)
    .bind(event_id)
    .bind(tenant_id)
    .bind(payment_intent_id)
    .bind(payment_attempt_id)
    .bind(payload_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn set_tenant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
