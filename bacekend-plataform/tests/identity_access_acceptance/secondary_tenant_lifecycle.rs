use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct SecondaryTenantLifecycleContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub operator_id: Uuid,
    pub second_tenant_id: Uuid,
}

pub async fn verify(context: SecondaryTenantLifecycleContext<'_>) {
    let SecondaryTenantLifecycleContext {
        client,
        migration_pool,
        http_addr,
        origin,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        operator_id,
        second_tenant_id,
    } = context;
    // Exercise activation and both close transitions on a separate tenant.
    // platform_support must be denied; platform_security may activate and close.
    let second_tenant_initial_state: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(second_tenant_id)
            .fetch_one(migration_pool)
            .await
            .expect("second tenant lifecycle state is queryable");
    assert_eq!(
        second_tenant_initial_state,
        ("pending_verification".to_owned(), 1)
    );
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_support' WHERE id=$1")
        .bind(operator_id)
        .execute(migration_pool)
        .await
        .expect("platform support role is selected for activation denial");
    let support_activation = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Support role must not activate a tenant",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("support activation receives an HTTP response");
    assert_eq!(support_activation.status(), StatusCode::FORBIDDEN);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(migration_pool)
        .await
        .expect("platform security role is selected for activation");
    let security_activation = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Authorized platform security verification",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("platform security can activate a verified tenant");
    assert_eq!(security_activation.status(), StatusCode::OK);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_support' WHERE id=$1")
        .bind(operator_id)
        .execute(migration_pool)
        .await
        .expect("platform support role is selected for close denial");
    let support_close = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "closing",
            "reason": "Support role must not close a tenant",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("support close receives an HTTP response");
    assert_eq!(support_close.status(), StatusCode::FORBIDDEN);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(migration_pool)
        .await
        .expect("platform owner role is selected for tenant close");
    let owner_closing = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "closing",
            "reason": "Authorized platform security close",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("platform owner can start tenant close");
    assert_eq!(owner_closing.status(), StatusCode::OK);
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(migration_pool)
        .await
        .expect("platform security role is selected to finish tenant close");
    let security_closed = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "closed",
            "reason": "Authorized platform security final close",
            "expected_version": 3
        }))
        .send()
        .await
        .expect("platform security can finish tenant close");
    assert_eq!(security_closed.status(), StatusCode::OK);
    let second_tenant_closed_state: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(second_tenant_id)
            .fetch_one(migration_pool)
            .await
            .expect("closed tenant lifecycle state is queryable");
    assert_eq!(second_tenant_closed_state, ("closed".to_owned(), 4));
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(migration_pool)
        .await
        .expect("platform owner role is restored after lifecycle matrix");
}
