use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct TenantStatusLifecycleContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub operator_id: Uuid,
    pub created_tenant_id: Uuid,
}

pub async fn verify(context: TenantStatusLifecycleContext<'_>) {
    let TenantStatusLifecycleContext {
        client, migration_pool, http_addr, origin, matrix_operator_session_cookie,
        matrix_operator_csrf_cookie, operator_id, created_tenant_id,
    } = context;
    // platform_support may read tenants but must not suspend them. Verify that
    // the denial leaves both status and version unchanged.
    let support_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Support role must not change tenant lifecycle",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("support lifecycle mutation receives a response");
    assert_eq!(support_status_change.status(), StatusCode::FORBIDDEN);
    let status_after_support_denial: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("tenant state after support denial is queryable");
    assert_eq!(status_after_support_denial, ("active".to_owned(), 2));
    
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform security role is selected for lifecycle acceptance");
    let security_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Authorized platform security suspension",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("platform security can suspend a tenant");
    assert_eq!(security_status_change.status(), StatusCode::OK);
    
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform owner role is restored for lifecycle acceptance");
    let owner_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Authorized platform owner reactivation",
            "expected_version": 3
        }))
        .send()
        .await
        .expect("platform owner can reactivate a suspended tenant");
    assert_eq!(owner_status_change.status(), StatusCode::OK);
    
    let owner_suspend_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Authorized platform owner suspension",
            "expected_version": 4
        }))
        .send()
        .await
        .expect("platform owner can suspend a tenant");
    assert_eq!(owner_suspend_status_change.status(), StatusCode::OK);
    
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform security role is selected for lifecycle reactivation");
    let security_reactivation = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Authorized platform security reactivation",
            "expected_version": 5
        }))
        .send()
        .await
        .expect("platform security can reactivate a suspended tenant");
    assert_eq!(security_reactivation.status(), StatusCode::OK);
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform owner role is restored for remaining lifecycle acceptance");
}
