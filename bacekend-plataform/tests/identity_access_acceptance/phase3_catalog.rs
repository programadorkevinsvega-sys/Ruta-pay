use crate::identity_access_acceptance_support::{
    assert_http_failure_diagnostics, safe_diagnostics_for_correlation,
};
use chrono::Utc;
use reqwest::StatusCode;
use reqwest::header;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub struct Phase3CatalogContext<'a> {
    pub client: &'a reqwest::Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
    pub tenant_id: Uuid,
    pub captured_events: &'a Arc<Mutex<Vec<HashMap<String, String>>>>,
}

pub async fn configure(context: Phase3CatalogContext<'_>) {
    let Phase3CatalogContext {
        client,
        migration_pool,
        http_addr,
        origin,
        operator_session_cookie,
        operator_csrf_cookie,
        tenant_id,
        captured_events,
    } = context;
    let phase3_slug = format!("phase3-{}", Uuid::new_v4().simple());
    let phase3_plan = client
        .post(format!("http://{http_addr}/v1/platform/catalog/plans"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_plan_create")
        .json(&serde_json::json!({
            "slug": phase3_slug,
            "name": "Phase 3 Acceptance",
            "description": "Real BFF to gRPC catalog acceptance"
        }))
        .send()
        .await
        .expect("catalog plan creation traverses the real BFF and gRPC module");
    assert_eq!(phase3_plan.status(), StatusCode::CREATED);
    let phase3_plan_body: serde_json::Value = phase3_plan
        .json()
        .await
        .expect("catalog plan response is valid JSON");
    let phase3_plan_id = phase3_plan_body["plan_id"]
        .as_str()
        .expect("catalog plan id is returned")
        .to_owned();

    let phase3_effective_from = (Utc::now() - chrono::Duration::seconds(30)).to_rfc3339();
    let phase3_price = client
        .post(format!(
            "http://{http_addr}/v1/platform/catalog/plans/{phase3_plan_id}/prices"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_price_publish")
        .json(&serde_json::json!({
            "plan_id": phase3_plan_id,
            "currency": "USD",
            "amount_minor": 1200,
            "period_unit": "month",
            "interval_count": 1,
            "effective_from": phase3_effective_from,
            "effective_until": null
        }))
        .send()
        .await
        .expect("price publication traverses the real BFF and gRPC module");
    assert_eq!(phase3_price.status(), StatusCode::CREATED);
    let phase3_price_body: serde_json::Value = phase3_price
        .json()
        .await
        .expect("published price response is valid JSON");
    let phase3_price_id = phase3_price_body["price_version_id"]
        .as_str()
        .expect("published price version id is returned")
        .to_owned();

    let phase3_capabilities = client
        .post(format!("http://{http_addr}/v1/platform/entitlements/capabilities"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_capabilities")
        .json(&serde_json::json!({
            "price_version_id": phase3_price_id,
            "capabilities": [{
                "capability_key": "payments.monthly",
                "enabled": true,
                "limit_value": 2,
                "unit": "operations",
                "overage_policy": "deny",
                "environment": "all",
                "quota_period": "month"
            }]
        }))
        .send()
        .await
        .expect("capability configuration traverses the real BFF and gRPC module");
    assert_eq!(phase3_capabilities.status(), StatusCode::OK);
    let phase3_capabilities_body: serde_json::Value = phase3_capabilities
        .json()
        .await
        .expect("capability configuration response is valid JSON");
    assert_eq!(phase3_capabilities_body["capabilities_saved"], 1);

    let phase3_assignment = client
        .post(format!("http://{http_addr}/v1/platform/entitlements/assignments"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_assignment")
        .json(&serde_json::json!({
            "tenant_id": tenant_id.to_string(),
            "price_version_id": phase3_price_id,
            "effective_from": phase3_effective_from,
            "effective_until": null
        }))
        .send()
        .await
        .expect("tenant plan assignment traverses the real BFF and gRPC module");
    if phase3_assignment.status() != StatusCode::CREATED {
        let response_status = phase3_assignment.status();
        let response_body = phase3_assignment
            .text()
            .await
            .expect("assignment error body is readable");
        panic!(
            "assignment returned {response_status}; safe envelope={response_body}; safe diagnostics={:?}",
            safe_diagnostics_for_correlation(captured_events, "corr_phase3_assignment")
        );
    }

    let missing_tenant_correlation = "corr_phase3_assignment_missing_tenant";
    let missing_tenant = client
        .post(format!("http://{http_addr}/v1/platform/entitlements/assignments"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", missing_tenant_correlation)
        .header(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )
        .header("tracestate", "vendor=opaque")
        .json(&serde_json::json!({
            "tenant_id": Uuid::new_v4().to_string(),
            "price_version_id": phase3_price_id,
            "effective_from": phase3_effective_from,
            "effective_until": null
        }))
        .send()
        .await
        .expect("missing-tenant failure traverses the real BFF and gRPC module");
    let missing_tenant_status = missing_tenant.status();
    let response_correlation = missing_tenant
        .headers()
        .get("x-correlation-id")
        .and_then(|value| value.to_str().ok())
        .expect("error response preserves correlation header");
    assert_eq!(response_correlation, missing_tenant_correlation);
    let missing_tenant_body: serde_json::Value = missing_tenant
        .json()
        .await
        .expect("missing-tenant error response is valid JSON");
    assert_eq!(missing_tenant_status, StatusCode::NOT_FOUND);
    assert_eq!(
        missing_tenant_body["error"]["code"],
        "resource_not_found"
    );
    assert_eq!(missing_tenant_body["error"]["category"], "not_found");
    assert_eq!(
        missing_tenant_body["error"]["correlation_id"],
        missing_tenant_correlation
    );
    assert_eq!(
        missing_tenant_body["error"]["trace_id"],
        "4bf92f3577b34da6a3ce929d0e0e4736"
    );
    assert_http_failure_diagnostics(
        captured_events,
        missing_tenant_correlation,
        "resource_not_found",
        "not_found",
    );

    sqlx::raw_sql("CREATE FUNCTION entitlements_quotas.reject_acceptance_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'acceptance outbox failure' USING ERRCODE='23514', CONSTRAINT='phase3_reject_assignment_outbox', TABLE='outbox_events', COLUMN='event_id'; END $$; CREATE TRIGGER phase3_reject_assignment_outbox BEFORE INSERT ON entitlements_quotas.outbox_events FOR EACH ROW EXECUTE FUNCTION entitlements_quotas.reject_acceptance_outbox()")
        .execute(migration_pool)
        .await
        .expect("real PostgreSQL assignment failure injector is installed");
    let before: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM entitlements_quotas.tenant_plan_assignments WHERE tenant_id=$1), (SELECT count(*) FROM entitlements_quotas.audit_events WHERE tenant_id=$1 AND action='tenant.plan.assigned'), (SELECT count(*) FROM entitlements_quotas.outbox_events WHERE event_type='tenant.plan.assigned' AND payload->>'tenant_id'=$1::text)",
    )
    .bind(tenant_id)
    .fetch_one(migration_pool)
    .await
    .expect("assignment audit and Outbox state before failure are measurable");
    let assignment_before: Option<(Uuid, Option<chrono::DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id,effective_until FROM entitlements_quotas.tenant_plan_assignments WHERE tenant_id=$1 AND effective_until IS NULL",
    )
    .bind(tenant_id)
    .fetch_optional(migration_pool)
    .await
    .expect("current assignment state before failure is measurable");
    let persistence_correlation = "corr_phase3_assignment_persistence_failure";
    let failed_assignment = client
        .post(format!("http://{http_addr}/v1/platform/entitlements/assignments"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", persistence_correlation)
        .header(
            "traceparent",
            "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
        )
        .json(&serde_json::json!({
            "tenant_id": tenant_id.to_string(),
            "price_version_id": phase3_price_id,
            "effective_from": (Utc::now() - chrono::Duration::seconds(10)).to_rfc3339(),
            "effective_until": null
        }))
        .send()
        .await
        .expect("assignment persistence failure traverses the real BFF and PostgreSQL");
    sqlx::raw_sql("DROP TRIGGER phase3_reject_assignment_outbox ON entitlements_quotas.outbox_events; DROP FUNCTION entitlements_quotas.reject_acceptance_outbox()")
        .execute(migration_pool)
        .await
        .expect("real PostgreSQL assignment failure injector is removed");
    let failed_assignment_status = failed_assignment.status();
    let failed_assignment_body: serde_json::Value = failed_assignment
        .json()
        .await
        .expect("assignment persistence error is returned as a safe JSON envelope");
    assert_eq!(failed_assignment_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        failed_assignment_body["error"]["code"],
        "postgres_constraint_violation"
    );
    assert_eq!(
        failed_assignment_body["error"]["category"],
        "persistence"
    );
    assert_eq!(
        failed_assignment_body["error"]["retryable"],
        false
    );
    assert_eq!(
        failed_assignment_body["error"]["correlation_id"],
        persistence_correlation
    );
    assert_eq!(
        failed_assignment_body["error"]["trace_id"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    let response_body = failed_assignment_body.to_string();
    assert!(!response_body.contains("acceptance outbox failure"));
    assert!(!response_body.contains("INSERT INTO"));
    assert_http_failure_diagnostics(
        captured_events,
        persistence_correlation,
        "postgres_constraint_violation",
        "persistence",
    );
    let events = captured_events
        .lock()
        .expect("event capture lock")
        .clone();
    let postgres_failure = events
        .iter()
        .rev()
        .find(|event| {
            event.get("event.name").map(String::as_str) == Some("grpc.dependency.call.failed")
                && event.get("dependency").map(String::as_str) == Some("postgresql")
                && event.get("correlation_id").map(String::as_str)
                    == Some(persistence_correlation)
        })
        .expect("PostgreSQL failure logs safe database metadata");
    assert_eq!(postgres_failure.get("sqlstate").map(String::as_str), Some("23514"));
    assert_eq!(
        postgres_failure.get("constraint").map(String::as_str),
        Some("phase3_reject_assignment_outbox")
    );
    assert_eq!(
        postgres_failure.get("table").map(String::as_str),
        Some("outbox_events")
    );
    assert_eq!(
        postgres_failure.get("column").map(String::as_str),
        Some("event_id")
    );
    assert!(
        !postgres_failure
            .values()
            .any(|value| value.contains("acceptance outbox failure")),
        "raw database messages are not logged"
    );
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM entitlements_quotas.tenant_plan_assignments WHERE tenant_id=$1), (SELECT count(*) FROM entitlements_quotas.audit_events WHERE tenant_id=$1 AND action='tenant.plan.assigned'), (SELECT count(*) FROM entitlements_quotas.outbox_events WHERE event_type='tenant.plan.assigned' AND payload->>'tenant_id'=$1::text)",
    )
    .bind(tenant_id)
    .fetch_one(migration_pool)
    .await
    .expect("assignment audit and Outbox state after rollback are measurable");
    let assignment_after: Option<(Uuid, Option<chrono::DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id,effective_until FROM entitlements_quotas.tenant_plan_assignments WHERE tenant_id=$1 AND effective_until IS NULL",
    )
    .bind(tenant_id)
    .fetch_optional(migration_pool)
    .await
    .expect("current assignment state after rollback is measurable");
    assert_eq!(
        before, after,
        "assignment, audit and Outbox rows roll back together"
    );
    assert_eq!(
        assignment_before, assignment_after,
        "a failed replacement preserves the previous assignment"
    );
}
