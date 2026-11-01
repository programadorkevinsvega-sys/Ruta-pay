use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
for attempt in 0..9 {
    let response = ctx.client
        .post(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &ctx.origin)
        .header(
            "x-correlation-id",
            format!("corr_identity_rate_limit_{attempt}"),
        )
        .header("traceparent", &ctx.state.traceparent)
        .json(&serde_json::json!({"email": ctx.email, "password": "incorrect-password"}))
        .send()
        .await
        .expect("rate limited login response arrives");
    let expected_status = if attempt == 8 {
        StatusCode::TOO_MANY_REQUESTS
    } else {
        StatusCode::UNAUTHORIZED
    };
    assert_eq!(
        response.status(),
        expected_status,
        "unexpected status for rate limit attempt {attempt}"
    );
    if attempt == 8 {
        let rate_limit_error: serde_json::Value =
            response.json().await.expect("safe rate limit envelope");
        assert_eq!(rate_limit_error["error"]["code"], "identity_rate_limited");
        assert_eq!(
            rate_limit_error["error"]["correlation_id"],
            "corr_identity_rate_limit_8"
        );
    }
}
 let mut isolated_ip_counter = ctx.redis_manager.clone();
let _: u64 = redis::cmd("DEL")
    .arg(&ctx.state.ip_key)
    .query_async(&mut isolated_ip_counter)
    .await
    .expect("test-owned per-IP counter is reset before threshold acceptance");
for attempt in 0..51 {
    let response = ctx.client
        .post(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &ctx.origin)
        .header(
            "x-correlation-id",
            format!("corr_identity_ip_limit_{attempt}"),
        )
        .json(&serde_json::json!({
            "email": format!("unknown_{attempt}@example.test"),
            "password": "incorrect-password"
        }))
        .send()
        .await
        .expect("per-IP rate limit response arrives");
    let expected_status = if attempt == 50 {
        StatusCode::TOO_MANY_REQUESTS
    } else {
        StatusCode::UNAUTHORIZED
    };
    assert_eq!(
        response.status(),
        expected_status,
        "unexpected status for per-IP rate limit attempt {attempt}"
    );
    if attempt == 50 {
        let rate_limit_error: serde_json::Value =
            response.json().await.expect("safe per-IP limit envelope");
        assert_eq!(rate_limit_error["error"]["code"], "identity_rate_limited");
        assert_eq!(
            rate_limit_error["error"]["correlation_id"],
            "corr_identity_ip_limit_50"
        );
    }
}
 let rate_limit_events = ctx.captured_events.lock().expect("event capture lock").clone();
let rate_limit_module_failure = rate_limit_events
    .iter()
    .find(|event| {
        event
            .get("event.name")
            .is_some_and(|name| name.contains("grpc.module.request.failed"))
            && event
                .get("error_code")
                .is_some_and(|code| code.contains("identity_rate_limited"))
            && event
                .get("correlation_id")
                .is_some_and(|value| value.contains("corr_identity_rate_limit_8"))
    })
    .expect("causal rate limit event is recorded");
let rate_limit_http_closure = rate_limit_events
    .iter()
    .find(|event| {
        event
            .get("event.name")
            .is_some_and(|name| name.contains("http.request.completed"))
            && event
                .get("correlation_id")
                .is_some_and(|value| value.contains("corr_identity_rate_limit_8"))
    })
    .expect("rate limit HTTP closure event is recorded");
for field in ["correlation_id", "trace_id"] {
    assert_eq!(
        rate_limit_module_failure.get(field),
        rate_limit_http_closure.get(field)
    );
}
assert!(
    rate_limit_http_closure
        .get("error_code")
        .is_some_and(|code| code.contains("identity_rate_limited"))
);
assert!(
    rate_limit_http_closure
        .get("error_category")
        .is_some_and(|category| category.contains("rate_limit"))
);
 // The public registration endpoint is limited per normalized account and per peer IP.
// Invalid passwords still consume an attempt, preventing cheap validation probes from
// bypassing the protection around the expensive password-hashing path.
// Use a dedicated loopback source IP so earlier registration requests in this
// end-to-end scenario cannot consume the IP budget being asserted here.}
