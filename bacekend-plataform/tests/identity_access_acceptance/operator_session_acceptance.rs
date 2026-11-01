use platform_contracts::identity_access::{
    AuthorizeOperatorOperationRequest,
    identity_access_client::IdentityAccessClient,
};
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use tonic::transport::Channel;
use uuid::Uuid;

use super::tenant_session_acceptance::TenantSessionCookies;

pub struct OperatorSessionCookies {
    pub session_cookie: String,
    pub csrf_cookie: String,
}

pub async fn verify(
    client: &Client,
    http_addr: std::net::SocketAddr,
    origin: &str,
    operator_email: &str,
    operator_password: &str,
    operator_id: Uuid,
    authorization_client: &mut IdentityAccessClient<Channel>,
    migration_pool: &PgPool,
    tenant_cookies: &TenantSessionCookies,
) -> OperatorSessionCookies {
    let session_cookie = &tenant_cookies.session_cookie;
    let csrf_cookie = &tenant_cookies.csrf_cookie;
        let invalid_origin_operator_login = client
            .post(format!("http://{http_addr}/v1/platform/identity/session"))
            .header(header::ORIGIN, "https://attacker.example")
            .json(&serde_json::json!({
                "email": operator_email.clone(),
                "password": operator_password.clone()
            }))
            .send()
            .await
            .expect("untrusted Origin operator login receives an HTTP response");
        assert_eq!(
            invalid_origin_operator_login.status(),
            StatusCode::FORBIDDEN
        );
    
        let operator_login = client
            .post(format!("http://{http_addr}/v1/platform/identity/session"))
            .header(header::ORIGIN, &origin)
            .header("x-correlation-id", "corr_operator_login")
            .json(&serde_json::json!({
                "email": operator_email.clone(),
                "password": operator_password.clone()
            }))
            .send()
            .await
            .expect("operator login response arrives");
        assert_eq!(operator_login.status(), StatusCode::OK);
        let operator_cookies: Vec<String> = operator_login
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_owned))
            .collect();
        let operator_session_cookie = cookie_value(&operator_cookies, "payvia_platform_session");
        let operator_csrf_cookie = cookie_value(&operator_cookies, "payvia_platform_csrf");
    
        let authorization = authorization_client
            .authorize_operator_operation(AuthorizeOperatorOperationRequest {
                session_token: operator_session_cookie.clone(),
                csrf_token: operator_csrf_cookie.clone(),
                permission: "tenants.activate".to_owned(),
                correlation_id: "corr_operator_authorization".to_owned(),
            })
            .await
            .expect("operator permission is checked in the identity module")
            .into_inner();
        assert_eq!(authorization.operator_id, operator_id.to_string());
        let denied_authorization = authorization_client
            .authorize_operator_operation(AuthorizeOperatorOperationRequest {
                session_token: operator_session_cookie.clone(),
                csrf_token: operator_csrf_cookie.clone(),
                permission: "billing.refunds.issue".to_owned(),
                correlation_id: "corr_operator_authorization_denied".to_owned(),
            })
            .await
            .expect_err("unassigned permission is denied");
        assert_eq!(denied_authorization.code(), tonic::Code::PermissionDenied);
        let authorization_audit_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM identity_access.audit_events WHERE actor_id=$1 AND action='operator.authorization.checked' AND correlation_id IN ('corr_operator_authorization','corr_operator_authorization_denied')",
        )
        .bind(operator_id)
        .fetch_one(&migration_pool)
        .await
        .expect("authorization decisions are auditable");
        assert_eq!(authorization_audit_count, 2);
        let operator_session = client
            .get(format!("http://{http_addr}/v1/platform/identity/session"))
            .header(
                header::COOKIE,
                format!(
                    "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
                ),
            )
            .send()
            .await
            .expect("operator session validates through the BFF");
        assert_eq!(operator_session.status(), StatusCode::OK);
        let operator_session_used_for_tenant = client
            .delete(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!(
                    "payvia_tenant_session={operator_session_cookie}; payvia_tenant_csrf={operator_csrf_cookie}"
                ),
            )
            .header("x-csrf-token", &operator_csrf_cookie)
            .send()
            .await
            .expect("operator session cannot access tenant identity");
        assert_eq!(
            operator_session_used_for_tenant.status(),
            StatusCode::UNAUTHORIZED
        );
    OperatorSessionCookies { session_cookie: operator_session_cookie, csrf_cookie: operator_csrf_cookie }
}
