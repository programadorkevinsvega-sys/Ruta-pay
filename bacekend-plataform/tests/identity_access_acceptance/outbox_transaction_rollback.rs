use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(client: &Client, migration_pool: &PgPool, http_addr: SocketAddr, origin: &str) {
    // Force the Outbox insert to fail after the identity INSERT. The transaction
    // must roll back the identity and audit writes rather than leave partial state.
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION identity_access.reject_test_outbox_event() RETURNS trigger \
         LANGUAGE plpgsql AS $phase2$ BEGIN \
         IF NEW.correlation_id = 'corr_force_identity_outbox_failure' THEN \
           RAISE EXCEPTION 'forced acceptance failure'; \
         END IF; RETURN NEW; END $phase2$; \
         CREATE TRIGGER reject_test_outbox_event BEFORE INSERT ON identity_access.outbox_events \
         FOR EACH ROW EXECUTE FUNCTION identity_access.reject_test_outbox_event();",
    )
    .execute(migration_pool)
    .await
    .expect("temporary failure trigger is installed in the isolated acceptance database");
    let rollback_email = format!("rollback_{}@example.test", Uuid::new_v4().simple());
    let rollback_response = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, origin)
        .header("x-correlation-id", "corr_force_identity_outbox_failure")
        .json(&serde_json::json!({
            "email": rollback_email.clone(),
            "password": format!("Rollback-password-{}!", Uuid::new_v4().simple())
        }))
        .send()
        .await
        .expect("forced Outbox failure returns an HTTP response");
    assert_eq!(
        rollback_response.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let rollback_identity_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
    )
    .bind(rollback_email.to_ascii_lowercase())
    .fetch_one(migration_pool)
    .await
    .expect("rollback identity count is queryable");
    assert_eq!(rollback_identity_count, 0, "identity insert must roll back");
    let rollback_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.audit_events WHERE correlation_id='corr_force_identity_outbox_failure'",
    )
    .fetch_one(migration_pool)
    .await
    .expect("rollback audit count is queryable");
    assert_eq!(rollback_audit_count, 0, "audit insert must not be partial");
    let rollback_outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.outbox_events WHERE correlation_id='corr_force_identity_outbox_failure'",
    )
    .fetch_one(migration_pool)
    .await
    .expect("rollback Outbox count is queryable");
    assert_eq!(
        rollback_outbox_count, 0,
        "Outbox insert must not be partial"
    );
    sqlx::raw_sql(
        "DROP TRIGGER reject_test_outbox_event ON identity_access.outbox_events; \
         DROP FUNCTION identity_access.reject_test_outbox_event();",
    )
    .execute(migration_pool)
    .await
    .expect("temporary failure trigger is removed");
}
