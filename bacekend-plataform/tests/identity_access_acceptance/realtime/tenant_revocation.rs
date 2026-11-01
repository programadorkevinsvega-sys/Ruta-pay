use crate::identity_access_acceptance_context::AcceptanceContext;
use platform_contracts::realtime::SubscribeEventsRequest;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let matrix_operator_login = ctx.client
    .post(format!("http://{http_addr}/v1/platform/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_operator_matrix_login")
    .json(&serde_json::json!({
        "email": operator_email.clone(),
        "password": operator_password.clone()
    }))
    .send()
    .await
    .expect("operator matrix login response arrives");
assert_eq!(matrix_operator_login.status(), StatusCode::OK);
let matrix_operator_cookies: Vec<String> = matrix_operator_login
    .headers()
    .get_all(header::SET_COOKIE)
    .iter()
    .filter_map(|value| value.to_str().ok().map(str::to_owned))
    .collect();
ctx.state.ctx.state.matrix_operator_session_cookie =
    cookie_value(&matrix_operator_cookies, "payvia_platform_session");
ctx.state.ctx.state.matrix_operator_csrf_cookie =
    cookie_value(&matrix_operator_cookies, "payvia_platform_csrf");
 let rejected_csrf = ctx.client
    .delete(format!("http://{http_addr}/v1/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-csrf-token", "wrong-csrf-token")
    .send()
    .await
    .expect("CSRF rejection response arrives");
assert_eq!(rejected_csrf.status(), StatusCode::FORBIDDEN);
 let mut tenant_realtime_subscription_client = ctx.realtime_acceptance_client.clone();
let mut tenant_realtime_subscription = tenant_realtime_subscription_client
    .subscribe_events(SubscribeEventsRequest {
        session_token: ctx.state.session_cookie.clone(),
        csrf_token: ctx.state.csrf_cookie.clone(),
        audience: "tenant".to_owned(),
        tenant_id: ctx.state.created_tenant_id.to_string(),
        after_cursor: ctx.state.replay_cursor,
        correlation_id: "corr_realtime_tenant_revocation_subscription".to_owned(),
    })
    .await
    .expect("authorized tenant can subscribe to its realtime stream")
    .into_inner();
 let logout = ctx.client
    .delete(format!("http://{http_addr}/v1/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.csrf_cookie)
    .header("x-correlation-id", "corr_identity_logout")
    .send()
    .await
    .expect("valid logout response arrives");
assert_eq!(logout.status(), StatusCode::NO_CONTENT);
let tenant_revocation_status = tokio::time::timeout(Duration::from_secs(20), async {
    loop {
        match tenant_realtime_subscription.message().await {
            Err(status) => break status,
            Ok(Some(_event)) => continue,
            Ok(None) => {
                panic!("tenant realtime subscription must report authorization revocation")
            }
        }
    }
})
.await
.expect("tenant realtime subscription revalidates session revocation");
assert_eq!(
    tenant_revocation_status.code(),
    tonic::Code::Unauthenticated
);
let tenant_logout_audited: bool = sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'identity.session.revoked' AND correlation_id = 'corr_identity_logout')",
)
.bind(ctx.state.registered_user_id)
.fetch_one(&ctx.migration_pool)
.await
.expect("tenant session revocation audit is queryable");
assert!(
    tenant_logout_audited,
    "tenant session revocation is auditable"
);
let replay = ctx.client
    .delete(format!("http://{http_addr}/v1/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header(
        header::COOKIE,
        format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
    )
    .header("x-csrf-token", &ctx.state.csrf_cookie)
    .send()
    .await
    .expect("revoked session replay is rejected");
assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}
