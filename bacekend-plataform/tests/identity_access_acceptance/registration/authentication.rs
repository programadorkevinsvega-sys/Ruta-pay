use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};
use sha2::{Digest, Sha256};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
ctx.state.ctx.state.trace_id = "4bf92f3577b34da6a3ce929d0e0e4736".to_owned();
ctx.state.ctx.state.traceparent = format!("00-{trace_id}-00f067aa0ba902b7-01");
let wrong_login = ctx.client
    .post(format!("http://{http_addr}/v1/identity/session"))
    .header(header::ORIGIN, &ctx.origin)
    .header("x-correlation-id", "corr_identity_wrong_password")
    .header("traceparent", &ctx.state.traceparent)
    .json(&serde_json::json!({"email": ctx.email, "password": "incorrect-password"}))
    .send()
    .await
    .expect("invalid password gets a classified response");
assert_eq!(wrong_login.status(), StatusCode::UNAUTHORIZED);
let error_body: serde_json::Value = wrong_login.json().await.expect("safe error envelope");
assert_eq!(error_body["error"]["code"], "authentication_failed");
assert_eq!(
    error_body["error"]["correlation_id"],
    "corr_identity_wrong_password"
);
assert_eq!(error_body["error"]["trace_id"], ctx.state.trace_id);
 let account_digest = Sha256::digest(ctx.email.to_ascii_lowercase().as_bytes());
ctx.state.ctx.state.account_key = format!(
    "platform:identity:login:account:{}",
    URL_SAFE_NO_PAD.encode(account_digest)
);
let ip_digest = Sha256::digest(ctx.test_source_ip.to_string().as_bytes());
ctx.state.ctx.state.ip_key = format!(
    "platform:identity:login:ip:{}",
    URL_SAFE_NO_PAD.encode(ip_digest)
);
let mut redis_connection = ctx.redis_manager.clone();
for key in [&ctx.state.account_key, &ctx.state.ip_key] {
    let ttl: i64 = redis::cmd("TTL")
        .arg(key)
        .query_async(&mut redis_connection)
        .await
        .expect("real Redis returns the rate-limit counter TTL");
    assert!(
        (1..=60).contains(&ttl),
        "rate-limit Redis counter must expire within its configured window"
    );
}
 let events = ctx.captured_events.lock().expect("event capture lock").clone();
let module_failure = events
    .iter()
    .find(|event| {
        event
            .get("event.name")
            .is_some_and(|name| name.contains("grpc.module.request.failed"))
            && event
                .get("operation")
                .is_some_and(|operation| operation.contains("create_tenant_session"))
    })
    .expect("causal identity failure event");
let http_closure = events
    .iter()
    .find(|event| {
        event
            .get("event.name")
            .is_some_and(|name| name.contains("http.request.completed"))
            && event
                .get("correlation_id")
                .is_some_and(|value| value.contains("corr_identity_wrong_password"))
    })
    .expect("correlated HTTP closure event");
for field in ["correlation_id", "trace_id"] {
    assert_eq!(module_failure.get(field), http_closure.get(field));
}
assert!(
    http_closure
        .get("error_code")
        .is_some_and(|code| code.contains("authentication_failed"))
);
assert!(
    http_closure
        .get("error_category")
        .is_some_and(|category| category.contains("authentication"))
);
}
