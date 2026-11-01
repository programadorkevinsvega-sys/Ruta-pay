use reqwest::{Client, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct TenantRegistrationResult {
    pub legal_identification_number: String,
    pub tenant_id: Uuid,
    pub tenant_body: Value,
    pub tenant_slug: String,
}

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
) -> TenantRegistrationResult {
    let legal_identification_number = format!(
        "9{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let tenant_slug = format!("acceptance-{}", Uuid::new_v4().simple());
    sqlx::raw_sql(
            "CREATE OR REPLACE FUNCTION tenant_registry.reject_test_outbox_event() RETURNS trigger \
             LANGUAGE plpgsql AS $tenant_register$ BEGIN \
             IF NEW.correlation_id = 'corr_tenant_registration_outbox_failure' THEN \
                 RAISE EXCEPTION 'injected tenant Outbox failure'; \
             END IF; RETURN NEW; END $tenant_register$; \
             CREATE TRIGGER reject_test_outbox_event BEFORE INSERT ON tenant_registry.outbox_events \
             FOR EACH ROW EXECUTE FUNCTION tenant_registry.reject_test_outbox_event();",
        )
        .execute(&migration_pool)
        .await
        .expect("tenant Outbox failure trigger is installed");
    let tenants_before_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count before Outbox failure is queryable");
    let tenant_outbox_failure = client
            .post(format!("http://{http_addr}/v1/tenants"))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .header("x-correlation-id", "corr_tenant_registration_outbox_failure")
            .json(&serde_json::json!({
                "display_name": "Tenant rollback acceptance",
                "slug": format!("rollback-{}", Uuid::new_v4().simple()),
                "legal_identification_type": "nit",
                "legal_identification_number": format!("7{:018}", Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000)
            }))
            .send()
            .await
            .expect("tenant registration Outbox failure receives an HTTP response");
    assert_eq!(
        tenant_outbox_failure.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let tenants_after_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count after Outbox failure is queryable");
    assert_eq!(
        tenants_after_outbox_failure, tenants_before_outbox_failure,
        "tenant registration must roll back when its Outbox write fails"
    );
    let failed_tenant_outbox_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tenant_registry.outbox_events WHERE correlation_id='corr_tenant_registration_outbox_failure'",
        )
        .fetch_one(&migration_pool)
        .await
        .expect("failed tenant Outbox write leaves no row");
    assert_eq!(failed_tenant_outbox_rows, 0);
    sqlx::raw_sql(
            "DROP TRIGGER reject_test_outbox_event ON tenant_registry.outbox_events;          DROP FUNCTION tenant_registry.reject_test_outbox_event();",
        )
        .execute(&migration_pool)
        .await
        .expect("tenant registration failure trigger is removed");
    let tenant_create = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_tenant_onboarding")
        .json(&serde_json::json!({
            "display_name": "Acceptance tenant",
            "slug": tenant_slug.clone(),
            "legal_identification_type": "nit",
            "legal_identification_number": legal_identification_number.clone()
        }))
        .send()
        .await
        .expect("tenant onboarding reaches tenant registry through BFF and gRPC");
    assert_eq!(tenant_create.status(), StatusCode::CREATED);
    let tenant_body: serde_json::Value = tenant_create
        .json()
        .await
        .expect("tenant onboarding returns a safe tenant representation");
    assert!(tenant_body.get("legal_identification_number").is_none());
    assert!(tenant_body.get("legal_identification_type").is_none());
    let created_tenant_id = Uuid::parse_str(
        tenant_body["tenant_id"]
            .as_str()
            .expect("created tenant identifier is present"),
    )
    .expect("created tenant identifier is a UUID");
    assert_eq!(tenant_body["status"], "pending_verification");
    let stored_identification: String = sqlx::query_scalar(
        "SELECT legal_identification_number FROM tenant_registry.tenants WHERE id=$1",
    )
    .bind(created_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant registry persists the submitted legal identification");
    assert_eq!(stored_identification, legal_identification_number);
    let tenant_event_payload: String = sqlx::query_scalar(
            "SELECT payload::text FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
        )
        .bind(created_tenant_id)
        .fetch_one(&migration_pool)
        .await
        .expect("tenant registration outbox event exists");
    assert!(!tenant_event_payload.contains(&legal_identification_number));
    let owner_membership_event: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered' AND correlation_id='corr_tenant_onboarding')",
        )
        .bind(created_tenant_id)
        .fetch_one(&migration_pool)
        .await
        .expect("tenant onboarding is durably queued for owner membership provisioning");
    assert!(owner_membership_event);
    let conflicting_document = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_duplicate_tenant_document")
        .json(&serde_json::json!({
            "display_name": "Another acceptance tenant",
            "slug": format!("another-acceptance-{}", Uuid::new_v4().simple()),
            "legal_identification_type": "nit",
            "legal_identification_number": legal_identification_number.clone()
        }))
        .send()
        .await
        .expect("duplicate legal identification returns a conflict");
    assert_eq!(conflicting_document.status(), StatusCode::CONFLICT);
    let conflict_envelope: serde_json::Value = conflicting_document
        .json()
        .await
        .expect("duplicate legal identification returns a safe error envelope");
    assert_eq!(
        conflict_envelope["error"]["correlation_id"],
        "corr_duplicate_tenant_document"
    );
    assert!(
        !conflict_envelope
            .to_string()
            .contains(&legal_identification_number)
    );
    TenantRegistrationResult {
        legal_identification_number,
        tenant_id: created_tenant_id,
        tenant_body,
        tenant_slug,
    }
}
