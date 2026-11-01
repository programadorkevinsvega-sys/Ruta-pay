use redis::aio::ConnectionManager;
use reqwest::{Client, StatusCode, header};
use std::{collections::HashMap, net::Ipv4Addr, sync::{Arc, Mutex}};
use uuid::Uuid;

pub struct TenantSessionCookies {
    pub session_cookie: String,
    pub csrf_cookie: String,
}

pub async fn verify(
    client: &Client,
    http_addr: std::net::SocketAddr,
    origin: &str,
    email: &str,
    password: &str,
    test_source_ip: Ipv4Addr,
    redis_manager: &ConnectionManager,
    captured_events: &Arc<Mutex<Vec<HashMap<String, String>>>>,
    registered_user_id: Uuid,
) -> TenantSessionCookies {
        let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
        let traceparent = format!("00-{trace_id}-00f067aa0ba902b7-01");
        let wrong_login = client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, &origin)
            .header("x-correlation-id", "corr_identity_wrong_password")
            .header("traceparent", &traceparent)
            .json(&serde_json::json!({"email": email, "password": "incorrect-password"}))
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
        assert_eq!(error_body["error"]["trace_id"], trace_id);
    
        let account_digest = Sha256::digest(email.to_ascii_lowercase().as_bytes());
        let account_key = format!(
            "platform:identity:login:account:{}",
            URL_SAFE_NO_PAD.encode(account_digest)
        );
        let ip_digest = Sha256::digest(test_source_ip.to_string().as_bytes());
        let ip_key = format!(
            "platform:identity:login:ip:{}",
            URL_SAFE_NO_PAD.encode(ip_digest)
        );
        let mut redis_connection = redis_manager.clone();
        for key in [&account_key, &ip_key] {
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
    
        let events = captured_events.lock().expect("event capture lock").clone();
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
    
        let login = client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, &origin)
            .header("x-correlation-id", "corr_identity_login")
            .json(&serde_json::json!({"email": email, "password": password}))
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
        let session_cookie = cookie_value(&cookies, "payvia_tenant_session");
        let csrf_cookie = cookie_value(&cookies, "payvia_tenant_csrf");
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
    
        let tenant_session = client
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
            registered_user_id.to_string()
        );
    
        let tenant_session_used_for_operator = client
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
    TenantSessionCookies { session_cookie, csrf_cookie }
}
