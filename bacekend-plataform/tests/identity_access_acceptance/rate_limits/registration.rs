use crate::identity_access_acceptance_context::AcceptanceContext;
use reqwest::{Client, header, StatusCode};
use uuid::Uuid;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
let rate_limit_source_id = Uuid::new_v4();
let rate_limit_source_bytes = rate_limit_source_id.as_bytes();
let rate_limit_source_ip = std::net::Ipv4Addr::new(
    127,
    rate_limit_source_bytes[0],
    rate_limit_source_bytes[1],
    rate_limit_source_bytes[2].max(1),
);
let rate_limit_client = Client::builder()
    .local_address(std::net::IpAddr::V4(rate_limit_source_ip))
    .ctx.default_headers.clone()(ctx.default_headers.clone())
    .build()
    .expect("isolated rate-limit HTTP client builds");
let limited_email = format!("limited_{}@example.test", Uuid::new_v4().simple());
for attempt in 0..5 {
    let response = rate_limit_client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &ctx.origin)
        .json(&serde_json::json!({
            "email": limited_email,
            "password": "long-lowercase-password"
        }))
        .send()
        .await
        .expect("registration attempt reaches the BFF");
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "attempt {attempt} is below the per-account registration limit"
    );
}
let account_limited = rate_limit_client
    .post(format!("http://{http_addr}/v1/identity/register"))
    .header(header::ORIGIN, &ctx.origin)
    .json(&serde_json::json!({
        "email": limited_email,
        "password": "long-lowercase-password"
    }))
    .send()
    .await
    .expect("account-limited registration response arrives");
assert_eq!(account_limited.status(), StatusCode::TOO_MANY_REQUESTS);
 // A separate set of accounts from the same peer proves the independent IP ceiling.
for attempt in 6..20 {
    let response = rate_limit_client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &ctx.origin)
        .json(&serde_json::json!({
            "email": format!("ip_limit_{attempt}_{}@example.test", Uuid::new_v4().simple()),
            "password": "long-lowercase-password"
        }))
        .send()
        .await
        .expect("IP-limited registration attempt reaches the BFF");
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "IP attempt {attempt} is below the per-IP registration limit"
    );
}
let ip_limited = rate_limit_client
    .post(format!("http://{http_addr}/v1/identity/register"))
    .header(header::ORIGIN, &ctx.origin)
    .json(&serde_json::json!({
        "email": format!("ip_limit_final_{}@example.test", Uuid::new_v4().simple()),
        "password": "long-lowercase-password"
    }))
    .send()
    .await
    .expect("IP-limited registration response arrives");
assert_eq!(ip_limited.status(), StatusCode::TOO_MANY_REQUESTS);
 let event_values: Vec<String> = ctx.captured_events
    .lock()
    .expect("event capture lock")
    .iter()
    .flat_map(|event| event.values().cloned())
    .collect();
assert!(!event_values.iter().any(|value| value.contains(&ctx.password)));
assert!(
    !event_values
        .iter()
        .any(|value| value.contains(&ctx.operator_password))
);
assert!(
    !event_values
        .iter()
        .any(|value| value.contains(&ctx.state.session_cookie))
);
assert!(
    !event_values
        .iter()
        .any(|value| value.contains(&ctx.state.csrf_cookie))
);
assert!(
    !event_values
        .iter()
        .any(|value| value.contains(&ctx.state.operator_session_cookie))
);
assert!(
    !event_values
        .iter()
        .any(|value| value.contains(&ctx.state.operator_csrf_cookie))
);
assert!(
    !event_values
        .iter()
        .any(|value| value.contains(&ctx.state.legal_identification_number))
);
assert!(
    !event_values
        .iter()
        .any(|value| value.contains(&ctx.state.second_legal_identification_number))
);
 // Exercise operator grants and real tenant administration endpoints for
// platform_owner, platform_security, and platform_support.}
