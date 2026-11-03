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
async fn payment_intents_http_path_enforces_auth_scope_isolation_and_idempotency() {
    let database = database::TestDatabase::new().await;
    let tenant_id = Uuid::new_v4();
    let other_tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity_access.tenant_users (id,email_normalized,password_hash,status) VALUES ($1,$2,'acceptance-only','active')",
    )
    .bind(user_id)
    .bind(format!("phase4-{}@example.test", Uuid::new_v4()))
    .execute(&database.migration_pool)
    .await
    .expect("API key owner fixture is persisted");

    let create_secret = insert_api_key(
        &database.migration_pool,
        tenant_id,
        user_id,
        "phase4-create",
        "test",
        &["payment_intents.create", "payment_intents.read"],
        "active",
    )
    .await;
    let read_only_secret = insert_api_key(
        &database.migration_pool,
        tenant_id,
        user_id,
        "phase4-read-only",
        "test",
        &["payment_intents.read"],
        "active",
    )
    .await;
    let revoked_secret = insert_api_key(
        &database.migration_pool,
        tenant_id,
        user_id,
        "phase4-revoked",
        "test",
        &["payment_intents.create", "payment_intents.read"],
        "revoked",
    )
    .await;
    let live_secret = insert_api_key(
        &database.migration_pool,
        tenant_id,
        user_id,
        "phase4-live",
        "live",
        &["payment_intents.create", "payment_intents.read"],
        "active",
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
    let base_url = format!("http://{http_addr}/v1/tenants/{tenant_id}/payment-intents");
    let payload = serde_json::json!({
        "amount_minor": 1299,
        "currency": "COP",
        "payment_method_type": "card",
        "merchant_reference": "order-phase4-001"
    });

    let unauthenticated = client
        .post(&base_url)
        .header("idempotency-key", "phase4-http-unauthenticated")
        .json(&payload)
        .send()
        .await
        .expect("unauthenticated request reaches the BFF");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let missing_key = client
        .post(&base_url)
        .bearer_auth(&create_secret)
        .json(&payload)
        .send()
        .await
        .expect("request without idempotency key reaches the BFF");
    assert_eq!(missing_key.status(), StatusCode::BAD_REQUEST);

    let created = client
        .post(&base_url)
        .bearer_auth(&create_secret)
        .header("idempotency-key", "phase4-http-create-001")
        .header("x-correlation-id", "phase4-http-create")
        .json(&payload)
        .send()
        .await
        .expect("valid payment intent traverses HTTP, BFF, gRPC, identity and PostgreSQL");
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body: serde_json::Value = created.json().await.expect("valid created response");
    let payment_intent_id = created_body["payment_intent_id"]
        .as_str()
        .expect("payment intent identifier is present")
        .to_owned();
    assert_eq!(created_body["status"], "created");
    assert_eq!(created_body["tenant_id"], tenant_id.to_string());

    let replay = client
        .post(&base_url)
        .bearer_auth(&create_secret)
        .header("idempotency-key", "phase4-http-create-001")
        .json(&payload)
        .send()
        .await
        .expect("same HTTP request is safely replayed");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body: serde_json::Value = replay.json().await.expect("valid replay response");
    assert_eq!(replay_body["payment_intent_id"], payment_intent_id);

    let changed_payload = serde_json::json!({
        "amount_minor": 1300,
        "currency": "COP",
        "payment_method_type": "card",
        "merchant_reference": "order-phase4-001"
    });
    let conflict = client
        .post(&base_url)
        .bearer_auth(&create_secret)
        .header("idempotency-key", "phase4-http-create-001")
        .json(&changed_payload)
        .send()
        .await
        .expect("changed request cannot reuse a committed idempotency key");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let read = client
        .get(format!("{base_url}/{payment_intent_id}"))
        .bearer_auth(&create_secret)
        .send()
        .await
        .expect("authorized tenant can read its payment intent");
    assert_eq!(read.status(), StatusCode::OK);

    let cross_tenant = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{other_tenant_id}/payment-intents/{payment_intent_id}"
        ))
        .bearer_auth(&create_secret)
        .send()
        .await
        .expect("API key cannot be used under another tenant route");
    assert_eq!(cross_tenant.status(), StatusCode::UNAUTHORIZED);

    let wrong_environment = client
        .get(format!("{base_url}/{payment_intent_id}"))
        .bearer_auth(&live_secret)
        .send()
        .await
        .expect("live key cannot read a test environment payment intent");
    assert_eq!(wrong_environment.status(), StatusCode::NOT_FOUND);

    let denied_scope = client
        .post(&base_url)
        .bearer_auth(&read_only_secret)
        .header("idempotency-key", "phase4-http-scope-denied")
        .json(&payload)
        .send()
        .await
        .expect("read-only API key is denied payment intent creation");
    assert_eq!(denied_scope.status(), StatusCode::FORBIDDEN);

    let revoked = client
        .post(&base_url)
        .bearer_auth(&revoked_secret)
        .header("idempotency-key", "phase4-http-revoked")
        .json(&payload)
        .send()
        .await
        .expect("revoked API key is rejected");
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

    let invalid_amount = client
        .post(&base_url)
        .bearer_auth(&create_secret)
        .header("idempotency-key", "phase4-http-invalid-amount")
        .json(&serde_json::json!({
            "amount_minor": 0,
            "currency": "COP",
            "payment_method_type": "card",
            "merchant_reference": null
        }))
        .send()
        .await
        .expect("invalid amount is rejected through the HTTP path");
    assert_eq!(invalid_amount.status(), StatusCode::BAD_REQUEST);

    let intent_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payments.payment_intents WHERE tenant_id=$1")
            .bind(tenant_id)
            .fetch_one(&database.migration_pool)
            .await
            .expect("intent count is queryable");
    let idempotency_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payments.idempotency_records WHERE tenant_id=$1")
            .bind(tenant_id)
            .fetch_one(&database.migration_pool)
            .await
            .expect("idempotency count is queryable");
    let outbox_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payments.outbox_events WHERE aggregate_id=$1")
            .bind(Uuid::parse_str(&payment_intent_id).expect("response intent id is a UUID"))
            .fetch_one(&database.migration_pool)
            .await
            .expect("outbox count is queryable");
    let leaked_secret_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.outbox_events WHERE payload::text LIKE '%' || $1 || '%'",
    )
    .bind(&create_secret)
    .fetch_one(&database.migration_pool)
    .await
    .expect("outbox payloads are queryable for redaction assertion");

    assert_eq!(
        intent_count, 1,
        "only the valid request creates a Payment Intent"
    );
    assert_eq!(
        idempotency_count, 1,
        "only the valid request reserves its idempotency key"
    );
    assert_eq!(outbox_count, 1, "creation writes exactly one durable event");
    assert_eq!(
        leaked_secret_count, 0,
        "raw API key never enters the Outbox payload"
    );

    let _ = http_shutdown.send(());
    http_task
        .await
        .expect("BFF task joins")
        .expect("BFF server stops");
    let _ = runtime_shutdown.send(());
    runtime_task
        .await
        .expect("runtime task joins")
        .expect("runtime server stops");
    database.finish().await;
}

async fn insert_api_key(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    name: &str,
    environment: &str,
    scopes: &[&str],
    status: &str,
) -> String {
    let secret = format!("{environment}_{}", Uuid::new_v4().simple());
    let digest = Sha256::digest(secret.as_bytes());
    let prefix = secret[..13].to_owned();
    let scopes = scopes
        .iter()
        .map(|scope| (*scope).to_owned())
        .collect::<Vec<_>>();
    sqlx::query(
        "INSERT INTO identity_access.api_keys (id,tenant_id,name,environment,prefix,secret_digest,scopes,status,created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(name)
    .bind(environment)
    .bind(prefix)
    .bind(digest.as_slice())
    .bind(scopes)
    .bind(status)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("API key fixture is persisted as a digest");
    secret
}
