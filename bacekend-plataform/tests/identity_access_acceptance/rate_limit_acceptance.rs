use reqwest::{Client, StatusCode, header};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

pub struct RateLimitAcceptanceContext<'a> {
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub default_headers: header::HeaderMap,
    pub captured_events: &'a Arc<Mutex<Vec<HashMap<String, String>>>>,
    pub password: &'a str,
    pub operator_password: &'a str,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
    pub legal_identification_number: &'a str,
    pub second_legal_identification_number: &'a str,
}

pub async fn verify(context: RateLimitAcceptanceContext<'_>) {
    let RateLimitAcceptanceContext {
        http_addr,
        origin,
        default_headers,
        captured_events,
        password,
        operator_password,
        session_cookie,
        csrf_cookie,
        operator_session_cookie,
        operator_csrf_cookie,
        legal_identification_number,
        second_legal_identification_number,
    } = context;
    // The public registration endpoint is limited per normalized account and per peer IP.
    // Invalid passwords still consume an attempt, preventing cheap validation probes from
    // bypassing the protection around the expensive password-hashing path.
    // Use a dedicated loopback source IP so earlier registration requests in this
    // end-to-end scenario cannot consume the IP budget being asserted here.
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
        .default_headers(default_headers)
        .build()
        .expect("isolated rate-limit HTTP client builds");
    let limited_email = format!("limited_{}@example.test", Uuid::new_v4().simple());
    for attempt in 0..5 {
        let response = rate_limit_client
            .post(format!("http://{http_addr}/v1/identity/register"))
            .header(header::ORIGIN, origin)
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
        .header(header::ORIGIN, origin)
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
            .header(header::ORIGIN, origin)
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
        .header(header::ORIGIN, origin)
        .json(&serde_json::json!({
            "email": format!("ip_limit_final_{}@example.test", Uuid::new_v4().simple()),
            "password": "long-lowercase-password"
        }))
        .send()
        .await
        .expect("IP-limited registration response arrives");
    assert_eq!(ip_limited.status(), StatusCode::TOO_MANY_REQUESTS);

    let event_values: Vec<String> = captured_events
        .lock()
        .expect("event capture lock")
        .iter()
        .flat_map(|event| event.values().cloned())
        .collect();
    assert!(!event_values.iter().any(|value| value.contains(&password)));
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&operator_password))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(session_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(csrf_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(operator_session_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(operator_csrf_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&legal_identification_number))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&second_legal_identification_number))
    );
}
