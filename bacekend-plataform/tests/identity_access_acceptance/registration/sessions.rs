use crate::identity_access_acceptance_context::AcceptanceContext;
use crate::identity_access_acceptance_support::cookie_value;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let login = ctx.client
    .post(format!("http://{http_addr}/v1/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_identity_login")
    .json(&serde_json::json!({"email": ctx.email, "password": ctx.password}))
    .send()
    .await
    .expect("valid login response arrives");
assert_eq!(login.status(), StatusCode::OK);
let cookies: Vec<String> = login
    .headers()
    .get_all(header::SET_COOKIE)
    .iter()
    .filter_map(|value| value.to_str().ok().map(str::to_owned))
    .collect();
ctx.state.ctx.state.session_cookie = cookie_value(&cookies, "payvia_tenant_session");
ctx.state.ctx.state.csrf_cookie = cookie_value(&cookies, "payvia_tenant_csrf");
assert!(
    cookies
        .iter()
        .any(|value| value.starts_with("payvia_tenant_session=")
            && value.contains("HttpOnly")
            && value.contains("Secure"))
);
assert!(
    cookies
        .iter()
        .any(|value| value.starts_with("payvia_tenant_csrf=") && value.contains("Secure"))
);
 let tenant_session = ctx.client
    .get(format!("http://{http_addr}/v1/identity/session"))
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .send()
    .await
    .expect("tenant session can be recovered after app reload");
assert_eq!(tenant_session.status(), StatusCode::OK);
let tenant_session_body: serde_json::Value = tenant_session
    .json()
    .await
    .expect("tenant session response contains authorized identity metadata");
assert_eq!(
    tenant_session_body["user_id"],
    ctx.state.registered_user_id.to_string()
);
 let tenant_session_used_for_operator = ctx.client
    .get(format!("http://{http_addr}/v1/platform/identity/session"))
    .header(
        header::COOKIE,
        format!("payvia_platform_session={session_cookie}; payvia_platform_csrf={csrf_cookie}"),
    )
    .send()
    .await
    .expect("tenant session cannot access operator identity");
assert_eq!(
    tenant_session_used_for_operator.status(),
    StatusCode::UNAUTHORIZED
);
 let invalid_origin_operator_login = ctx.client
    .post(format!("http://{http_addr}/v1/platform/identity/session"))
    .header(header::ORIGIN, "https://attacker.example")
    .json(&serde_json::json!({
        "email": ctx.operator_email.clone(),
        "password": ctx.operator_password.clone()
    }))
    .send()
    .await
    .expect("untrusted Origin operator login receives an HTTP response");
assert_eq!(
    invalid_origin_operator_login.status(),
    StatusCode::FORBIDDEN
);
 let operator_login = ctx.client
    .post(format!("http://{http_addr}/v1/platform/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_operator_login")
    .json(&serde_json::json!({
        "email": ctx.operator_email.clone(),
        "password": ctx.operator_password.clone()
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
ctx.state.ctx.state.operator_session_cookie = cookie_value(&operator_cookies, "payvia_platform_session");
ctx.state.ctx.state.operator_csrf_cookie = cookie_value(&operator_cookies, "payvia_platform_csrf");
 let authorization = ctx.authorization_client
    .authorize_operator_operation(AuthorizeOperatorOperationRequest {
        session_token: ctx.state.operator_session_cookie.clone(),
        csrf_token: ctx.state.operator_csrf_cookie.clone(),
        permission: "tenants.activate".to_owned(),
        correlation_id: "corr_operator_authorization".to_owned(),
    })
    .await
    .expect("operator permission is checked in the identity module")
    .into_inner();
assert_eq!(authorization.ctx.operator_id, ctx.operator_id.to_string());
let denied_authorization = ctx.authorization_client
    .authorize_operator_operation(AuthorizeOperatorOperationRequest {
        session_token: ctx.state.operator_session_cookie.clone(),
        csrf_token: ctx.state.operator_csrf_cookie.clone(),
        permission: "billing.refunds.issue".to_owned(),
        correlation_id: "corr_operator_authorization_denied".to_owned(),
    })
    .await
    .expect_err("unassigned permission is denied");
assert_eq!(denied_authorization.code(), tonic::Code::PermissionDenied);
let authorization_audit_count: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM identity_access.audit_events WHERE actor_id=$1 AND action='operator.authorization.checked' AND correlation_id IN ('corr_operator_authorization','corr_operator_authorization_denied')",
)
.bind(ctx.operator_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("authorization decisions are auditable");
assert_eq!(authorization_audit_count, 2);
let operator_session = ctx.client
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
let operator_session_used_for_tenant = ctx.client
    .delete(format!("http://{http_addr}/v1/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!(
            "payvia_tenant_session={operator_session_cookie}; payvia_tenant_csrf={operator_csrf_cookie}"
        ),
    )
    .header("x-csrf-token", &ctx.state.operator_csrf_cookie)
    .send()
    .await
    .expect("operator session cannot access tenant identity");
assert_eq!(
    operator_session_used_for_tenant.status(),
    StatusCode::UNAUTHORIZED
);}
