use super::valid_origin;
use axum::http::{HeaderMap, HeaderValue, header};

fn origin_headers(origin: &str, host: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_str(origin).expect("valid origin header"),
    );
    headers.insert(
        header::HOST,
        HeaderValue::from_str(host).expect("valid host header"),
    );
    headers
}

#[test]
fn accepts_exact_loopback_http_and_exact_https_origins() {
    for authority in ["localhost:4173", "127.0.0.1:4173", "[::1]:4173"] {
        assert!(valid_origin(&origin_headers(
            &format!("http://{authority}"),
            authority,
        )));
    }
    assert!(valid_origin(&origin_headers(
        "https://dashboard.payvia.example",
        "dashboard.payvia.example",
    )));
}

#[test]
fn rejects_origin_mismatch_and_host_suffix_impostors() {
    assert!(!valid_origin(&origin_headers(
        "http://localhost.evil.example:4173",
        "localhost.evil.example:4173",
    )));
    assert!(!valid_origin(&origin_headers(
        "http://127.0.0.1:4173",
        "localhost:4173",
    )));
    assert!(!valid_origin(&origin_headers(
        "http://example.test",
        "example.test",
    )));
}
