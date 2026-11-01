use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    registered_user_id: uuid::Uuid,
) {
    // Two simultaneous registrations for the same normalized email must have
    // exactly one winner, even when both requests pass the rate limiter.
    let concurrent_email = format!("concurrent_{}@example.test", Uuid::new_v4().simple());
    let concurrent_password = format!("Concurrent-password-{}!", Uuid::new_v4().simple());
    let concurrent_url = format!("http://{http_addr}/v1/identity/register");
    let first_request = client
        .post(&concurrent_url)
        .header(header::ORIGIN, origin)
        .header("x-correlation-id", "corr_identity_concurrent_first")
        .json(&serde_json::json!({
            "email": concurrent_email.clone(),
            "password": concurrent_password.clone()
        }))
        .send();
    let second_request = client
        .post(&concurrent_url)
        .header(header::ORIGIN, origin)
        .header("x-correlation-id", "corr_identity_concurrent_second")
        .json(&serde_json::json!({
            "email": concurrent_email.clone(),
            "password": concurrent_password.clone()
        }))
        .send();
    let (first_result, second_result) = tokio::join!(first_request, second_request);
    let first_status = first_result
        .expect("first concurrent registration response arrives")
        .status();
    let second_status = second_result
        .expect("second concurrent registration response arrives")
        .status();
    let mut concurrent_statuses = vec![first_status, second_status];
    concurrent_statuses.sort();
    assert_eq!(
        concurrent_statuses,
        vec![StatusCode::CREATED, StatusCode::CONFLICT],
        "one concurrent registration succeeds and the other conflicts"
    );
    let concurrent_identity_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
    )
    .bind(concurrent_email.to_ascii_lowercase())
    .fetch_one(migration_pool)
    .await
    .expect("concurrent identity count is queryable");
    assert_eq!(concurrent_identity_count, 1);

    let registration_event: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.outbox_events WHERE aggregate_id = $1 AND event_type = 'identity.tenant_user.registered')",
    )
    .bind(registered_user_id)
    .fetch_one(migration_pool)
    .await
    .expect("registration outbox event is queryable");
    assert!(
        registration_event,
        "registration is written to durable outbox"
    );
    let registration_audit: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'tenant_user.registered' AND correlation_id = 'corr_identity_register')",
    )
    .bind(registered_user_id)
    .fetch_one(migration_pool)
    .await
    .expect("registration audit event is queryable");
    assert!(registration_audit, "registration is auditable");
}
