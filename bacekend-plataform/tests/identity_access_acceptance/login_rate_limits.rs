use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use redis::aio::ConnectionManager;
use reqwest::{Client, StatusCode, header};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use uuid::Uuid;

pub async fn verify(
    redis_manager: &ConnectionManager,
    http_addr: SocketAddr,
    origin: &str,
    traceparent: &str,
    email: &str,
    ip_key: &str,
) {
    let source_id = Uuid::new_v4();
    let source_bytes = source_id.as_bytes();
    let source_ip = Ipv4Addr::new(
        127,
        source_bytes[0],
        source_bytes[1].max(1),
        source_bytes[2].max(1),
    );
    let mut default_headers = header::HeaderMap::new();
    default_headers.insert(
        header::HOST,
        format!("localhost:{}", http_addr.port())
            .parse()
            .expect("acceptance host header is valid"),
    );
    let rate_limit_client = Client::builder()
        .local_address(IpAddr::V4(source_ip))
        .default_headers(default_headers)
        .build()
        .expect("isolated login rate-limit client builds");
    let rate_limit_email = format!("login_limit_{}@example.test", Uuid::new_v4().simple());
        assert_ne!(
            rate_limit_email, email,
            "rate-limit test account is isolated from the acceptance tenant"
        );
    let account_digest = Sha256::digest(rate_limit_email.to_ascii_lowercase().as_bytes());
    let account_key = format!(
        "platform:identity:login:account:{}",
        URL_SAFE_NO_PAD.encode(account_digest)
    );
    let ip_digest = Sha256::digest(source_ip.to_string().as_bytes());
    let isolated_ip_key = format!(
        "platform:identity:login:ip:{}",
        URL_SAFE_NO_PAD.encode(ip_digest)
    );
    assert_ne!(
        isolated_ip_key, ip_key,
        "rate-limit test uses a peer IP isolated from earlier acceptance requests"
    );
    let mut redis_connection = redis_manager.clone();
    let _: u64 = redis::cmd("DEL")
        .arg(&account_key)
        .arg(&isolated_ip_key)
        .query_async(&mut redis_connection)
        .await
        .expect("test-owned login rate-limit counters are reset");

    let trace_id = traceparent
        .split('-')
        .nth(1)
        .expect("acceptance traceparent contains a trace ID");
    for attempt in 0..11 {
        let correlation_id = format!("corr_identity_rate_limit_{attempt}");
        let response = rate_limit_client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, origin)
            .header("x-correlation-id", &correlation_id)
            .header("traceparent", traceparent)
            .json(&serde_json::json!({
                "email": rate_limit_email,
                "password": "incorrect-password"
            }))
            .send()
            .await
            .expect("rate-limited login response arrives");
        let (expected_status, expected_code, expected_category) = if attempt == 10 {
            (
                StatusCode::TOO_MANY_REQUESTS,
                "identity_rate_limited",
                "rate_limit",
            )
        } else {
            (
                StatusCode::UNAUTHORIZED,
                "authentication_failed",
                "authentication",
            )
        };
        assert_error_response(
            response,
            expected_status,
            expected_code,
            expected_category,
            &correlation_id,
            trace_id,
            "per-account login limit",
        )
        .await;
    }

    let mut isolated_ip_counter = redis_manager.clone();
    let _: u64 = redis::cmd("DEL")
        .arg(&isolated_ip_key)
        .query_async(&mut isolated_ip_counter)
        .await
        .expect("test-owned per-IP counter is reset before threshold acceptance");
    for attempt in 0..51 {
        let correlation_id = format!("corr_identity_ip_limit_{attempt}");
        let response = rate_limit_client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, origin)
            .header("x-correlation-id", &correlation_id)
            .header("traceparent", traceparent)
            .json(&serde_json::json!({
                "email": format!("unknown_{attempt}_{}@example.test", Uuid::new_v4().simple()),
                "password": "incorrect-password"
            }))
            .send()
            .await
            .expect("per-IP rate-limit response arrives");
        let (expected_status, expected_code, expected_category) = if attempt == 50 {
            (
                StatusCode::TOO_MANY_REQUESTS,
                "identity_rate_limited",
                "rate_limit",
            )
        } else {
            (
                StatusCode::UNAUTHORIZED,
                "authentication_failed",
                "authentication",
            )
        };
        assert_error_response(
            response,
            expected_status,
            expected_code,
            expected_category,
            &correlation_id,
            trace_id,
            "per-IP login limit",
        )
        .await;
    }

    let mut redis_connection = redis_manager.clone();
    let _: u64 = redis::cmd("DEL")
        .arg(&account_key)
        .arg(&isolated_ip_key)
        .query_async(&mut redis_connection)
        .await
        .expect("test-owned login rate-limit counters are cleaned up");

}

async fn assert_error_response(
    response: reqwest::Response,
    expected_status: StatusCode,
    expected_code: &str,
    expected_category: &str,
    correlation_id: &str,
    trace_id: &str,
    scenario: &str,
) {
    let actual_status = response.status();
    let body = response
        .text()
        .await
        .expect("login error response body is readable");
    assert_eq!(
        actual_status, expected_status,
        "{scenario} returned unexpected status for {correlation_id}; safe envelope: {body}"
    );
    let body: serde_json::Value =
        serde_json::from_str(&body).expect("login error response is a safe JSON envelope");
    assert_eq!(body["error"]["code"], expected_code);
    assert_eq!(body["error"]["category"], expected_category);
    assert_eq!(body["error"]["correlation_id"], correlation_id);
    assert_eq!(body["error"]["trace_id"], trace_id);
    assert_eq!(body["error"]["retryable"], false);
}
