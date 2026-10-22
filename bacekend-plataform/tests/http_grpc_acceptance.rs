use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use platform_contracts::identity_access::identity_access_client::IdentityAccessClient as IdentityAccessGrpcClient;
use platform_contracts::realtime::realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient;
use platform_contracts::runtime::runtime_probe_client::RuntimeProbeClient;
use platform_contracts::tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, sync::oneshot};
use tracing::{
    Event,
    field::{Field, Visit},
};
use tracing_subscriber::{
    layer::{Context, Layer},
    prelude::*,
    registry::LookupSpan,
};

#[derive(Clone, Default)]
struct EventCapture(Arc<Mutex<Vec<HashMap<String, String>>>>);

impl<S> Layer<S> for EventCapture
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut fields = HashMap::new();
        event.record(&mut FieldCapture(&mut fields));
        if let Some(span) = context.lookup_current() {
            fields.insert("current_span".to_owned(), span.name().to_owned());
        }
        self.0.lock().expect("event capture lock").push(fields);
    }
}

struct FieldCapture<'a>(&'a mut HashMap<String, String>);

impl Visit for FieldCapture<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

#[tokio::test]
async fn real_http_bff_and_grpc_module_cover_success_unavailable_and_timeout() {
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    let captured = EventCapture::default();
    let capture_store = captured.0.clone();
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("acceptance")))
        .with(captured)
        .try_init()
        .expect("acceptance subscriber initializes once");

    let runtime_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("runtime bind");
    let runtime_addr = runtime_listener.local_addr().expect("runtime address");
    let module_identity_client = IdentityAccessGrpcClient::new(
        tonic::transport::Endpoint::from_shared(format!("http://{runtime_addr}"))
            .expect("identity module endpoint parses")
            .connect_lazy(),
    );
    let (runtime_shutdown, runtime_signal) = oneshot::channel();
    let database_pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(
            &std::env::var("PLATFORM_TEST_DATABASE_ADMIN_URL")
                .expect("isolated PostgreSQL test administrator URL is configured"),
        )
        .expect("isolated PostgreSQL pool parses");
    let redis_url = std::env::var("PLATFORM_REDIS_URL").expect("real Redis test URL is configured");
    let redis_client = redis::Client::open(redis_url).expect("Redis test URL parses");
    let redis_manager = redis::aio::ConnectionManager::new(redis_client.clone())
        .await
        .expect("real Redis connection is available");
    let runtime_task = tokio::spawn(platform_runtime::module_config::serve(
        runtime_listener,
        database_pool,
        redis_manager,
        redis_client,
        module_identity_client,
        async move {
            let _ = runtime_signal.await;
        },
    ));
    let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{runtime_addr}"))
        .expect("valid grpc endpoint");
    let grpc_client = RuntimeProbeClient::connect(endpoint)
        .await
        .expect("grpc connection");
    let identity_endpoint =
        tonic::transport::Endpoint::from_shared(format!("http://{runtime_addr}"))
            .expect("valid identity gRPC endpoint");
    let identity_client = IdentityAccessGrpcClient::connect(identity_endpoint)
        .await
        .expect("identity gRPC connection");
    let tenant_client = TenantRegistryGrpcClient::connect(format!("http://{runtime_addr}"))
        .await
        .expect("tenant registry gRPC connection");
    let realtime_client = RealtimeGatewayGrpcClient::connect(format!("http://{runtime_addr}"))
        .await
        .expect("realtime gRPC connection");

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

    let blackhole_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("blackhole listener bind");
    let blackhole_addr = blackhole_listener
        .local_addr()
        .expect("blackhole listener address");
    let (connection_accepted_sender, connection_accepted_receiver) = oneshot::channel();
    let (blackhole_shutdown_sender, blackhole_shutdown_receiver) = oneshot::channel();
    let blackhole_task = tokio::spawn(async move {
        let (socket, _) = blackhole_listener
            .accept()
            .await
            .expect("gRPC transport connects to blackhole listener");
        connection_accepted_sender
            .send(())
            .expect("acceptance test is waiting for connection");
        let shutdown_result = blackhole_shutdown_receiver.await;
        drop(shutdown_result);
        drop(socket);
    });
    let blackhole_endpoint =
        tonic::transport::Endpoint::from_shared(format!("http://{blackhole_addr}"))
            .expect("valid blackhole gRPC endpoint");
    let blackhole_grpc_client = RuntimeProbeClient::new(blackhole_endpoint.connect_lazy());
    let blackhole_realtime_client =
        RealtimeGatewayGrpcClient::new(blackhole_endpoint.connect_lazy());
    let blackhole_tenant_client = TenantRegistryGrpcClient::new(
        tonic::transport::Endpoint::from_shared(format!("http://{blackhole_addr}"))
            .expect("blackhole tenant endpoint parses")
            .connect_lazy(),
    );
    let timeout_http_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("timeout BFF bind");
    let timeout_http_addr = timeout_http_listener
        .local_addr()
        .expect("timeout BFF address");
    let (timeout_http_shutdown, timeout_http_signal) = oneshot::channel();
    let timeout_http_task = tokio::spawn(async move {
        axum::serve(
            timeout_http_listener,
            payvia_platform_bff::http::routes::router(
                blackhole_grpc_client,
                timeout_identity_client,
                blackhole_tenant_client,
                blackhole_realtime_client,
            )
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = timeout_http_signal.await;
        })
        .await
    });

    capture_store.lock().expect("event capture lock").clear();
    let timeout_correlation_id = "corr_timeout_001";
    let timeout_request = {
        let client = client.clone();
        let timeout_url = format!("http://{timeout_http_addr}/health/ready");
        let timeout_traceparent = traceparent.clone();
        tokio::spawn(async move {
            client
                .get(timeout_url)
                .header("x-correlation-id", timeout_correlation_id)
                .header("traceparent", timeout_traceparent)
                .header("authorization", "Bearer sentinel-sensitive-token")
                .send()
                .await
        })
    };
    connection_accepted_receiver
        .await
        .expect("gRPC transport reached the non-responsive TCP peer");
    let timeout_started = std::time::Instant::now();
    let timed_out = timeout_request
        .await
        .expect("timeout HTTP task joins")
        .expect("BFF returns a classified timeout response");
    let timeout_elapsed = timeout_started.elapsed();
    assert!(
        timeout_elapsed >= std::time::Duration::from_millis(1500),
        "the BFF must wait for its configured dependency deadline"
    );
    assert_eq!(timed_out.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        timed_out
            .headers()
            .get("x-correlation-id")
            .and_then(|value| value.to_str().ok()),
        Some(timeout_correlation_id)
    );
    let timeout_body: serde_json::Value = timed_out
        .json()
        .await
        .expect("valid timeout error envelope");
    assert_eq!(timeout_body["error"]["code"], "module_timeout");
    assert_eq!(timeout_body["error"]["category"], "timeout");
    assert_eq!(timeout_body["error"]["retryable"], true);
    assert_eq!(
        timeout_body["error"]["correlation_id"],
        timeout_correlation_id
    );
    assert_eq!(timeout_body["error"]["trace_id"], trace_id);

    let timeout_events = capture_store.lock().expect("event capture lock").clone();
    let dependency_failure = timeout_events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("bff.dependency.call.failed"))
        })
        .expect("timeout dependency failure event");
    let request_failure = timeout_events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("bff.request.failed"))
        })
        .expect("timeout request failure event");
    let timeout_closure = timeout_events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("http.request.completed"))
        })
        .expect("timeout HTTP completion event");
    for failure_event in [dependency_failure, request_failure] {
        for field in ["correlation_id", "trace_id"] {
            assert_eq!(
                failure_event.get(field),
                timeout_closure.get(field),
                "timeout failure and HTTP closure must share {field}"
            );
        }
        assert!(
            failure_event
                .get("error_category")
                .is_some_and(|category| category.contains("timeout"))
        );
    }
    assert!(
        timeout_closure
            .get("error_code")
            .is_some_and(|code| code.contains("module_timeout"))
    );
    assert!(
        timeout_closure
            .get("error_category")
            .is_some_and(|category| category.contains("timeout"))
    );
    assert_eq!(
        timeout_closure.get("correlation_id").map(String::as_str),
        Some(timeout_correlation_id)
    );
    assert!(
        !timeout_events
            .iter()
            .flat_map(|event| event.values())
            .any(|value| value.contains("sentinel-sensitive-token")),
        "timeout diagnostics must redact authorization credentials"
    );

    timeout_http_shutdown
        .send(())
        .expect("timeout BFF is waiting for graceful shutdown");
    timeout_http_task
        .await
        .expect("timeout BFF task joins")
        .expect("timeout BFF shuts down");
    blackhole_shutdown_sender
        .send(())
        .expect("blackhole listener is waiting for shutdown");
    blackhole_task.await.expect("blackhole task joins");
    provider.shutdown().expect("tracer provider shuts down");
}
