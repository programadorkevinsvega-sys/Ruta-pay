use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    email: &str,
    password: &str,
    registered_user_id: uuid::Uuid,
) {
    // Duplicate normalized email must be rejected at the HTTP boundary without
    // creating a second identity or a second durable registration event.
    let duplicate_registration = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, origin)
        .header("x-correlation-id", "corr_identity_duplicate_email")
        .json(&serde_json::json!({
            "email": format!(" {} ", email.to_ascii_uppercase()),
            "password": password
        }))
        .send()
        .await
        .expect("normalized duplicate email registration receives a classified response");
    assert_eq!(duplicate_registration.status(), StatusCode::CONFLICT);
    let duplicate_envelope: serde_json::Value = duplicate_registration
        .json()
        .await
        .expect("duplicate registration returns a safe error envelope");
    assert_eq!(duplicate_envelope["error"]["code"], "identity_conflict");
    let duplicate_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
    )
    .bind(email.to_ascii_lowercase())
    .fetch_one(migration_pool)
    .await
    .expect("duplicate identity count is queryable");
    assert_eq!(duplicate_count, 1, "normalized email remains unique");
    let duplicate_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.outbox_events WHERE aggregate_id=$1 AND event_type='identity.tenant_user.registered'",
    )
    .bind(registered_user_id)
    .fetch_one(migration_pool)
    .await
    .expect("registration event count is queryable");
    assert_eq!(
        duplicate_event_count, 1,
        "duplicate registration emits no event"
    );
}
