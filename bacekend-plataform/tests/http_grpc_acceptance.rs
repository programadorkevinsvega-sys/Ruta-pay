#[path = "support/http_grpc_acceptance.rs"]
mod http_grpc_acceptance_support;

use tokio::{net::TcpListener, sync::oneshot};

#[tokio::test]
async fn real_http_bff_and_grpc_module_cover_success_and_unavailable() {
    let (provider, capture_store) = http_grpc_acceptance_support::initialize_capture();
    let runtime = http_grpc_acceptance_support::start_runtime().await;
    let runtime_shutdown = runtime.shutdown;
    let runtime_task = runtime.task;
    let grpc_client = runtime.clients.grpc;
    let identity_client = runtime.clients.identity;
    let tenant_client = runtime.clients.tenant;
    let realtime_client = runtime.clients.realtime;
    let catalog_client = runtime.clients.catalog;
    let entitlements_client = runtime.clients.entitlements;

    let http_listener = TcpListener::bind("127.0.0.1:0").await.expect("BFF bind");
    let http_addr = http_listener.local_addr().expect("BFF address");
    let (http_shutdown, http_signal) = oneshot::channel();
    let timeout_identity_client = identity_client.clone();
    let http_task = tokio::spawn(async move {
        axum::serve(
            http_listener,
            payvia_platform_bff::http::routes::router(
                grpc_client,
                identity_client.clone(),
                tenant_client,
                realtime_client,
                catalog_client,
                entitlements_client,
            )
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = http_signal.await;
        })
        .await
    });
    let correlation_id = "corr_acceptance_001";
    let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
    let traceparent = format!("00-{trace_id}-00f067aa0ba902b7-01");
    let client = reqwest::Client::new();
    let success = client
        .get(format!("http://{http_addr}/health/ready"))
        .header("x-correlation-id", correlation_id)
        .header("traceparent", &traceparent)
        .header("authorization", "Bearer sentinel-sensitive-token")
        .send()
        .await
        .expect("real HTTP readiness request");
    assert_eq!(success.status(), reqwest::StatusCode::OK);
    assert_eq!(
        success
            .headers()
            .get("x-correlation-id")
            .and_then(|value| value.to_str().ok()),
        Some(correlation_id)
    );
    let success_body: serde_json::Value = success.json().await.expect("valid readiness JSON");
    assert_eq!(success_body["trace_id"], trace_id);

    let live = client
        .get(format!("http://{http_addr}/health/live"))
        .header("x-correlation-id", correlation_id)
        .send()
        .await
        .expect("liveness remains independent from module readiness");
    assert_eq!(live.status(), reqwest::StatusCode::OK);
    capture_store.lock().expect("event capture lock").clear();
    let _ = runtime_shutdown.send(());
    runtime_task
        .await
        .expect("runtime task joins")
        .expect("runtime shuts down");
    let failed = client
        .get(format!("http://{http_addr}/health/ready"))
        .header("x-correlation-id", correlation_id)
        .header("traceparent", &traceparent)
        .header("authorization", "Bearer sentinel-sensitive-token")
        .send()
        .await
        .expect("HTTP returns classified dependency failure");
    assert_eq!(failed.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let failed_body: serde_json::Value = failed.json().await.expect("valid error envelope");
    assert_eq!(failed_body["error"]["code"], "module_unavailable");
    assert_eq!(failed_body["error"]["category"], "dependency");
    assert_eq!(failed_body["error"]["correlation_id"], correlation_id);
    assert_eq!(failed_body["error"]["trace_id"], trace_id);

    let events = capture_store.lock().expect("event capture lock").clone();
    let causal = events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("bff.request.failed"))
        })
        .expect("causal failure event");
    let closure = events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("http.request.completed"))
        })
        .expect("HTTP completion event");
    for field in ["correlation_id", "trace_id"] {
        assert_eq!(
            causal.get(field),
            closure.get(field),
            "{field} must correlate"
        );
    }
    assert!(
        closure
            .get("error_code")
            .is_some_and(|code| code.contains("module_unavailable"))
    );
    assert!(
        closure
            .get("error_category")
            .is_some_and(|category| category.contains("dependency"))
    );
    assert_eq!(
        closure.get("correlation_id").map(String::as_str),
        Some("corr_acceptance_001")
    );
    assert!(
        closure
            .get("span_id")
            .is_some_and(|span_id| span_id.len() == 16 && span_id != "0000000000000000"),
        "HTTP completion event must expose the active span identifier"
    );
    assert!(
        causal
            .get("span_id")
            .is_some_and(|span_id| span_id.len() == 16 && span_id != "0000000000000000"),
        "failure event must expose its active span identifier"
    );
    assert!(
        !events
            .iter()
            .flat_map(|event| event.values())
            .any(|value| value.contains("sentinel-sensitive-token"))
    );

    let _ = http_shutdown.send(());
    http_task
        .await
        .expect("BFF task joins")
        .expect("BFF shuts down");

    provider.shutdown().expect("tracer provider shuts down");
}
