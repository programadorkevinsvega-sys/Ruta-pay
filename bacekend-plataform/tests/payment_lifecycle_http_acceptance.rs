#[path = "phase4_payments_acceptance/database.rs"]
mod database;
#[path = "support/http_grpc_runtime.rs"]
mod http_grpc_runtime;

use axum::serve;
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::{net::TcpListener, sync::oneshot};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn payment_attempt_and_transaction_apis_cross_http_bff_grpc_and_payments_module() {
    let database = database::TestDatabase::new().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity_access.tenant_users (id,email_normalized,password_hash,status) VALUES ($1,$2,'acceptance-only','active')",
    )
    .bind(user_id)
    .bind(format!("lifecycle-{}@example.test", Uuid::new_v4()))
    .execute(&database.migration_pool)
    .await
    .expect("API key owner is persisted");

    let full_scope_secret = insert_api_key(
        &database.migration_pool,
        tenant_id,
        user_id,
        "lifecycle-full",
        &[
            "payment_intents.create",
            "payment_intents.read",
            "payment_intents.update",
            "payment_attempts.create",
            "payment_attempts.read",
            "payment_transactions.read",
        ],
    )
    .await;
    let read_only_secret = insert_api_key(
        &database.migration_pool,
        tenant_id,
        user_id,
        "lifecycle-read-only",
        &["payment_intents.read", "payment_attempts.read"],
    )
    .await;

    let runtime = http_grpc_runtime::start_runtime_with_pool(database.runtime_pool.clone()).await;
    let runtime_shutdown = runtime.shutdown;
    let runtime_task = runtime.task;
    let clients = runtime.clients;
    let http_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("BFF HTTP listener binds");
    let http_addr = http_listener.local_addr().expect("BFF address");
    let (http_shutdown, http_signal) = oneshot::channel();
    let http_task = tokio::spawn(async move {
        serve(
            http_listener,
            payvia_platform_bff::http::routes::router_with_payments(
                clients.grpc,
                clients.identity,
                clients.tenant,
                clients.realtime,
                clients.catalog,
                clients.entitlements,
                clients.payments,
            ),
        )
        .with_graceful_shutdown(async move {
            let _ = http_signal.await;
        })
        .await
    });

    let client = Client::new();
    let intents_url = format!("http://{http_addr}/v1/tenants/{tenant_id}/payment-intents");
    let created_intent = client
        .post(&intents_url)
        .bearer_auth(&full_scope_secret)
        .header("idempotency-key", "lifecycle-http-intent-001")
        .json(&serde_json::json!({
            "amount_minor": 1299,
            "currency": "COP",
            "payment_method_type": "card",
            "merchant_reference": "lifecycle-http-order"
        }))
        .send()
        .await
        .expect("intent creation crosses HTTP, BFF, gRPC, identity and PostgreSQL");
    assert_eq!(created_intent.status(), StatusCode::CREATED);
    let intent_body: serde_json::Value = created_intent.json().await.expect("intent response JSON");
    let intent_id = intent_body["payment_intent_id"]
        .as_str()
        .expect("intent identifier is returned")
        .to_owned();

    let attempts_url = format!("{intents_url}/{intent_id}/attempts");
    let attempt_payload = serde_json::json!({
        "provider_code": "wompi",
        "provider_account_reference": "account_01"
    });
    let created_attempt = client
        .post(&attempts_url)
        .bearer_auth(&full_scope_secret)
        .header("idempotency-key", "lifecycle-http-attempt-001")
        .json(&attempt_payload)
        .send()
        .await
        .expect("attempt creation crosses all four application boundaries");
    assert_eq!(created_attempt.status(), StatusCode::CREATED);
    let attempt_body: serde_json::Value =
        created_attempt.json().await.expect("attempt response JSON");
    let attempt_id = attempt_body["attempt_id"]
        .as_str()
        .expect("attempt identifier is returned")
        .to_owned();
    assert_eq!(attempt_body["status"], "created");
    assert_eq!(attempt_body["attempt_number"], 1);

    let replay = client
        .post(&attempts_url)
        .bearer_auth(&full_scope_secret)
        .header("idempotency-key", "lifecycle-http-attempt-001")
        .json(&attempt_payload)
        .send()
        .await
        .expect("same HTTP attempt command replays its original result");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body: serde_json::Value = replay.json().await.expect("replay response JSON");
    assert_eq!(replay_body["attempt_id"], attempt_id);

    let listed_attempts = client
        .get(&attempts_url)
        .bearer_auth(&full_scope_secret)
        .send()
        .await
        .expect("attempt query crosses the complete HTTP path");
    assert_eq!(listed_attempts.status(), StatusCode::OK);
    let attempts_body: serde_json::Value =
        listed_attempts.json().await.expect("attempt list JSON");
    assert_eq!(attempts_body["attempts"].as_array().unwrap().len(), 1);

    let denied_attempt = client
        .post(&attempts_url)
        .bearer_auth(&read_only_secret)
        .header("idempotency-key", "lifecycle-http-attempt-denied")
        .json(&attempt_payload)
        .send()
        .await
        .expect("read-only API key is evaluated by the identity service");
    assert_eq!(denied_attempt.status(), StatusCode::FORBIDDEN);

    let status_url = format!("{intents_url}/{intent_id}/status");
    let processing = client
        .patch(&status_url)
        .bearer_auth(&full_scope_secret)
        .json(&serde_json::json!({
            "payment_attempt_id": attempt_id,
            "status": "processing",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("status update crosses HTTP, BFF, gRPC and payments module");
    assert_eq!(processing.status(), StatusCode::OK);
    let processing_body: serde_json::Value =
        processing.json().await.expect("status update response JSON");
    assert_eq!(processing_body["payment_intent"]["status"], "processing");
    assert_eq!(processing_body["aggregate_version"], 2);

    let current_intent = client
        .get(format!("{intents_url}/{intent_id}"))
        .bearer_auth(&full_scope_secret)
        .send()
        .await
        .expect("intent remains queryable after status transition");
    assert_eq!(current_intent.status(), StatusCode::OK);
    let current_intent: serde_json::Value =
        current_intent.json().await.expect("updated intent JSON");
    assert_eq!(current_intent["status"], "processing");

    let stale_update = client
        .patch(&status_url)
        .bearer_auth(&full_scope_secret)
        .json(&serde_json::json!({
            "payment_attempt_id": attempt_id,
            "status": "outcome_unknown",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("stale aggregate version is rejected");
    assert_eq!(stale_update.status(), StatusCode::CONFLICT);

    let denied_update = client
        .patch(&status_url)
        .bearer_auth(&read_only_secret)
        .json(&serde_json::json!({
            "payment_attempt_id": attempt_id,
            "status": "outcome_unknown",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("the update scope is checked before executing the use case");
    assert_eq!(denied_update.status(), StatusCode::FORBIDDEN);

    let forged_provider_result = client
        .patch(&status_url)
        .bearer_auth(&full_scope_secret)
        .json(&serde_json::json!({
            "payment_attempt_id": attempt_id,
            "status": "succeeded",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("tenant API keys cannot forge provider-confirmed results");
    assert_eq!(forged_provider_result.status(), StatusCode::BAD_REQUEST);

    let transaction_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payments.payment_transactions (id,tenant_id,environment,payment_intent_id,payment_attempt_id,provider_code,provider_account_reference,provider_transaction_reference,amount_minor,currency,status,aggregate_version) VALUES ($1,$2,'test',$3,$4,'wompi','account_01','lifecycle-http-provider-tx',1299,'COP','pending',1)",
    )
    .bind(transaction_id)
    .bind(tenant_id)
    .bind(Uuid::parse_str(&intent_id).expect("intent id is UUID"))
    .bind(Uuid::parse_str(&attempt_id).expect("attempt id is UUID"))
    .execute(&database.migration_pool)
    .await
    .expect("valid submitted attempt owns a transaction with matching amount and currency");

    let transactions_url = format!("{intents_url}/{intent_id}/transactions");
    let listed_transactions = client
        .get(&transactions_url)
        .bearer_auth(&full_scope_secret)
        .send()
        .await
        .expect("transaction query crosses the complete HTTP path");
    assert_eq!(listed_transactions.status(), StatusCode::OK);
    let transactions_body: serde_json::Value =
        listed_transactions.json().await.expect("transaction list JSON");
    assert_eq!(transactions_body["transactions"].as_array().unwrap().len(), 1);
    assert_eq!(
        transactions_body["transactions"][0]["transaction_id"],
        transaction_id.to_string()
    );

    let missing_transaction_scope = client
        .get(&transactions_url)
        .bearer_auth(&read_only_secret)
        .send()
        .await
        .expect("transaction read scope is independently enforced");
    assert_eq!(missing_transaction_scope.status(), StatusCode::FORBIDDEN);

    let timeout = client
        .patch(&status_url)
        .bearer_auth(&full_scope_secret)
        .json(&serde_json::json!({
            "payment_attempt_id": attempt_id,
            "status": "outcome_unknown",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("timeout status creates a reconciliation case");
    assert_eq!(timeout.status(), StatusCode::OK);
    let timeout_body: serde_json::Value =
        timeout.json().await.expect("timeout transition response JSON");
    assert_eq!(timeout_body["payment_intent"]["status"], "outcome_unknown");
    assert_eq!(timeout_body["aggregate_version"], 3);
    let reconciliation_cases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.payment_reconciliation_cases WHERE payment_intent_id=$1 AND payment_attempt_id=$2 AND status='pending'",
    )
    .bind(Uuid::parse_str(&intent_id).expect("intent id is UUID"))
    .bind(Uuid::parse_str(&attempt_id).expect("attempt id is UUID"))
    .fetch_one(&database.migration_pool)
    .await
    .expect("reconciliation case is durable");
    assert_eq!(reconciliation_cases, 1);

    let cross_tenant = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{}/payment-intents/{intent_id}/attempts",
            Uuid::new_v4()
        ))
        .bearer_auth(&full_scope_secret)
        .send()
        .await
        .expect("API key cannot be reused under a different tenant route");
    assert_eq!(cross_tenant.status(), StatusCode::UNAUTHORIZED);

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.audit_events WHERE resource_id IN ($1,$2)",
    )
    .bind(Uuid::parse_str(&intent_id).expect("intent id is UUID"))
    .bind(Uuid::parse_str(&attempt_id).expect("attempt id is UUID"))
    .fetch_one(&database.migration_pool)
    .await
    .expect("lifecycle audit rows are queryable");
    assert!(audit_count >= 5, "mutations and authorized reads are audited");

    let _ = http_shutdown.send(());
    http_task.await.expect("BFF task joins").expect("BFF stops");
    let _ = runtime_shutdown.send(());
    runtime_task.await.expect("runtime task joins").expect("runtime stops");
    database.finish().await;
}

async fn insert_api_key(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    name: &str,
    scopes: &[&str],
) -> String {
    let secret = format!("test_{}", Uuid::new_v4().simple());
    let digest = Sha256::digest(secret.as_bytes());
    let prefix = secret[..13].to_owned();
    let scopes = scopes.iter().map(|scope| (*scope).to_owned()).collect::<Vec<_>>();
    sqlx::query(
        "INSERT INTO identity_access.api_keys (id,tenant_id,name,environment,prefix,secret_digest,scopes,status,created_by) VALUES ($1,$2,$3,'test',$4,$5,$6,'active',$7)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(name)
    .bind(prefix)
    .bind(digest.as_slice())
    .bind(scopes)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("API key stores only its digest");
    secret
}
