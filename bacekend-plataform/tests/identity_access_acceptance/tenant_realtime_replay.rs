use reqwest::{Client, StatusCode, header};
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    http_addr: SocketAddr,
    session_cookie: &str,
    csrf_cookie: &str,
    created_tenant_id: Uuid,
) -> u64 {
    let replay_url = format!(
        "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id={created_tenant_id}"
    );
    let tenant_replay = client
        .get(format!("{replay_url}&cursor=0"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_realtime_tenant_replay")
        .send()
        .await
        .expect("tenant can recover realtime journal by authorized cursor");
    assert_eq!(tenant_replay.status(), StatusCode::OK);
    let replay_body: serde_json::Value = tenant_replay
        .json()
        .await
        .expect("authorized event replay is safe JSON");
    assert!(replay_body["events"].as_array().is_some_and(|events| {
        events
            .iter()
            .any(|event| event["event_type"] == "tenant.registered")
    }));
    let replay_cursor = replay_body["next_cursor"]
        .as_u64()
        .expect("cursor is unsigned");
    let cursor_replay = client
        .get(format!("{replay_url}&cursor={replay_cursor}"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("replay resumes at the acknowledged cursor");
    let cursor_replay_body: serde_json::Value = cursor_replay
        .json()
        .await
        .expect("cursor replay is safe JSON");
    assert_eq!(
        cursor_replay_body["events"].as_array().map(Vec::len),
        Some(0)
    );
    
    // A cursor ahead of the durable scope cursor is not an empty page: it is
    // an invalid checkpoint and must be rejected so clients do not skip events.
    let future_cursor = client
        .get(format!(
            "{replay_url}&cursor={}",
            replay_cursor.saturating_add(100)
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_realtime_future_cursor")
        .send()
        .await
        .expect("future realtime cursor receives a classified response");
    assert_eq!(future_cursor.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    
    for invalid_cursor in [
        "-1",
        "not-a-number",
        "9223372036854775808",
        "18446744073709551616",
    ] {
        let invalid_cursor_response = client
            .get(format!("{replay_url}&cursor={invalid_cursor}"))
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .send()
            .await
            .expect("malformed or negative realtime cursor receives an HTTP response");
        assert_eq!(
            invalid_cursor_response.status(),
            StatusCode::BAD_REQUEST,
            "invalid cursor {invalid_cursor} must be rejected at the HTTP boundary"
        );
    }
    let invalid_realtime_scope = client
        .get(format!(
            "http://{http_addr}/v1/realtime/events?audience=unknown&cursor=0"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("unsupported realtime audience receives an HTTP response");
    assert_eq!(invalid_realtime_scope.status(), StatusCode::BAD_REQUEST);
    let malformed_realtime_tenant = client
        .get(format!(
            "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id=not-a-uuid&cursor=0"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("malformed realtime tenant scope receives an HTTP response");
    assert_eq!(malformed_realtime_tenant.status(), StatusCode::BAD_REQUEST);
    
    let foreign_tenant_replay = client
        .get(format!(
            "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id={}&cursor=0",
            Uuid::new_v4()
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant realtime audience cannot cross tenant boundary");
    assert_eq!(foreign_tenant_replay.status(), StatusCode::FORBIDDEN);
    replay_cursor
}
