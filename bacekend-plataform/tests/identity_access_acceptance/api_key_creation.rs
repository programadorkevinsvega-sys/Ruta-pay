use reqwest::{Client, StatusCode, header};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    created_tenant_id: Uuid,
) -> (Uuid, String) {
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION identity_access.reject_test_api_key_outbox_event() RETURNS trigger \
         LANGUAGE plpgsql AS $api_key$ BEGIN \
         IF NEW.correlation_id = 'corr_api_key_outbox_failure' THEN \
             RAISE EXCEPTION 'injected API key Outbox failure'; \
         END IF; RETURN NEW; END $api_key$; \
         CREATE TRIGGER reject_test_api_key_outbox_event BEFORE INSERT ON identity_access.outbox_events \
         FOR EACH ROW EXECUTE FUNCTION identity_access.reject_test_api_key_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("API key Outbox failure trigger is installed");
    let api_keys_before_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count before Outbox failure is queryable");
    let api_key_outbox_failure = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_api_key_outbox_failure")
        .json(&serde_json::json!({
            "name": "Rollback API key",
            "environment": "test",
            "scopes": ["payment_intents.create"]
        }))
        .send()
        .await
        .expect("API key Outbox failure receives an HTTP response");
    assert_eq!(
        api_key_outbox_failure.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let api_keys_after_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count after Outbox failure is queryable");
    assert_eq!(
        api_keys_after_outbox_failure, api_keys_before_outbox_failure,
        "API key creation must roll back when its Outbox write fails"
    );
    let failed_api_key_audit_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.audit_events WHERE correlation_id='corr_api_key_outbox_failure'",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("failed API key transaction leaves no audit row");
    assert_eq!(failed_api_key_audit_rows, 0);
    sqlx::raw_sql(
        "DROP TRIGGER reject_test_api_key_outbox_event ON identity_access.outbox_events;          DROP FUNCTION identity_access.reject_test_api_key_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("API key failure trigger is removed");
    let api_key_creation = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_api_key_create")
        .json(&serde_json::json!({
            "name": "Acceptance live key",
            "environment": "live",
            "scopes": ["payment_intents.create", "usage.consume"]
        }))
        .send()
        .await
        .expect(
            "tenant owner can create an API key with payment and quota scopes through BFF and gRPC",
        );
    assert_eq!(api_key_creation.status(), StatusCode::CREATED);
    let api_key_created: serde_json::Value = api_key_creation
        .json()
        .await
        .expect("API key creation returns the one-time secret");
    let api_key_id = Uuid::parse_str(
        api_key_created["key_id"]
            .as_str()
            .expect("API key id is present"),
    )
    .expect("API key id is a UUID");
    let api_key_secret = api_key_created["secret_once"]
        .as_str()
        .expect("one-time API key secret is present");
    assert!(api_key_secret.starts_with("live_"));
    assert_eq!(
        api_key_created["prefix"].as_str(),
        Some(&api_key_secret[..13])
    );
    assert_eq!(api_key_created["environment"], "live");
    let stored_key_digest: Vec<u8> = sqlx::query_scalar(
        "SELECT secret_digest FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
    )
    .bind(created_tenant_id)
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("API key digest is persisted");
    assert_eq!(
        stored_key_digest.as_slice(),
        Sha256::digest(api_key_secret.as_bytes()).as_slice()
    );
    (api_key_id, api_key_secret.to_owned())
}
