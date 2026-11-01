use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;

pub struct TenantRegistrationRaceContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub second_session_cookie: &'a str,
    pub second_csrf_cookie: &'a str,
}

pub async fn verify(context: TenantRegistrationRaceContext<'_>) {
    let TenantRegistrationRaceContext {
        client,
        migration_pool,
        http_addr,
        origin,
        second_session_cookie,
        second_csrf_cookie,
    } = context;
    // Concurrent tenant registration with the same slug must commit exactly
    // one aggregate and one durable registration event.
    let concurrent_slug = format!("concurrent-slug-{}", Uuid::new_v4().simple());
    let concurrent_slug_document_a = format!(
        "7{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let concurrent_slug_document_b = format!(
        "7{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let concurrent_slug_url = format!("http://{http_addr}/v1/tenants");
    let concurrent_slug_cookie = format!(
        "payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"
    );
    let (concurrent_slug_a, concurrent_slug_b) = tokio::join!(
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_slug_race_a")
            .json(&serde_json::json!({
                "display_name": "Concurrent slug A",
                "slug": concurrent_slug,
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_slug_document_a
            }))
            .send(),
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_slug_race_b")
            .json(&serde_json::json!({
                "display_name": "Concurrent slug B",
                "slug": concurrent_slug,
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_slug_document_b
            }))
            .send()
    );
    let concurrent_slug_a = concurrent_slug_a.expect("first slug race response arrives");
    let concurrent_slug_b = concurrent_slug_b.expect("second slug race response arrives");
    let mut concurrent_slug_statuses = vec![concurrent_slug_a.status(), concurrent_slug_b.status()];
    concurrent_slug_statuses.sort();
    assert_eq!(
        concurrent_slug_statuses,
        vec![StatusCode::CREATED, StatusCode::CONFLICT],
        "concurrent slug registration has one winner and one conflict"
    );
    let concurrent_slug_winner: serde_json::Value =
        if concurrent_slug_a.status() == StatusCode::CREATED {
            concurrent_slug_a
                .json()
                .await
                .expect("winning slug registration returns tenant metadata")
        } else {
            concurrent_slug_b
                .json()
                .await
                .expect("winning slug registration returns tenant metadata")
        };
    let concurrent_slug_winner_id = Uuid::parse_str(
        concurrent_slug_winner["tenant_id"]
            .as_str()
            .expect("winning slug tenant id is present"),
    )
    .expect("winning slug tenant id is a UUID");
    let concurrent_slug_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants WHERE slug=$1")
            .bind(&concurrent_slug)
            .fetch_one(&migration_pool)
            .await
            .expect("concurrent slug row count is queryable");
    assert_eq!(concurrent_slug_rows, 1);
    let concurrent_slug_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
    )
    .bind(concurrent_slug_winner_id)
    .fetch_one(&migration_pool)
    .await
    .expect("concurrent slug Outbox count is queryable");
    assert_eq!(concurrent_slug_events, 1);

    // The legal-identity unique constraint must provide the same guarantee
    // when requests use different slugs but the same normalized document.
    let concurrent_document = format!(
        "6{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let (concurrent_document_a, concurrent_document_b) = tokio::join!(
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_document_race_a")
            .json(&serde_json::json!({
                "display_name": "Concurrent document A",
                "slug": format!("concurrent-document-a-{}", Uuid::new_v4().simple()),
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_document
            }))
            .send(),
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_document_race_b")
            .json(&serde_json::json!({
                "display_name": "Concurrent document B",
                "slug": format!("concurrent-document-b-{}", Uuid::new_v4().simple()),
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_document
            }))
            .send()
    );
    let concurrent_document_a =
        concurrent_document_a.expect("first legal identity race response arrives");
    let concurrent_document_b =
        concurrent_document_b.expect("second legal identity race response arrives");
    let mut concurrent_document_statuses = vec![
        concurrent_document_a.status(),
        concurrent_document_b.status(),
    ];
    concurrent_document_statuses.sort();
    assert_eq!(
        concurrent_document_statuses,
        vec![StatusCode::CREATED, StatusCode::CONFLICT],
        "concurrent legal identity registration has one winner and one conflict"
    );
    let concurrent_document_winner: serde_json::Value =
        if concurrent_document_a.status() == StatusCode::CREATED {
            concurrent_document_a
                .json()
                .await
                .expect("winning legal identity registration returns tenant metadata")
        } else {
            concurrent_document_b
                .json()
                .await
                .expect("winning legal identity registration returns tenant metadata")
        };
    let concurrent_document_winner_id = Uuid::parse_str(
        concurrent_document_winner["tenant_id"]
            .as_str()
            .expect("winning legal identity tenant id is present"),
    )
    .expect("winning legal identity tenant id is a UUID");
    let concurrent_document_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.tenants WHERE legal_identification_type='cc' AND legal_identification_number=$1",
    )
    .bind(&concurrent_document)
    .fetch_one(&migration_pool)
    .await
    .expect("concurrent legal identity row count is queryable");
    assert_eq!(concurrent_document_rows, 1);
    let concurrent_document_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
    )
    .bind(concurrent_document_winner_id)
    .fetch_one(&migration_pool)
    .await
    .expect("concurrent legal identity Outbox count is queryable");
    assert_eq!(concurrent_document_events, 1);
}
