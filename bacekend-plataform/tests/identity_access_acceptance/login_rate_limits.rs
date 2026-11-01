use redis::aio::ConnectionManager;
use reqwest::{Client, StatusCode, header};
use std::net::SocketAddr;

pub async fn verify(
    client: &Client,
    redis_manager: &ConnectionManager,
    http_addr: SocketAddr,
    origin: &str,
    traceparent: &str,
    email: &str,
    ip_key: &str,
) {
    for attempt in 0..9 {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, origin)
            .header(
                "x-correlation-id",
                format!("corr_identity_rate_limit_{attempt}"),
            )
            .header("traceparent", traceparent)
            .json(&serde_json::json!({"email": email, "password": "incorrect-password"}))
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

    let mut isolated_ip_counter = redis_manager.clone();
    let _: u64 = redis::cmd("DEL")
        .arg(&ip_key)
        .query_async(&mut isolated_ip_counter)
        .await
        .expect("test-owned per-IP counter is reset before threshold acceptance");
    for attempt in 0..51 {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, origin)
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
}
