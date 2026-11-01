#[path = "support/http_grpc_acceptance.rs"]
mod http_grpc_acceptance_support;
#[path = "support/http_grpc_timeout_assertions.rs"]
mod http_grpc_timeout_assertions;

use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient as CatalogPricingGrpcClient,
    entitlements_quotas::entitlements_quotas_client::EntitlementsQuotasClient as EntitlementsQuotasGrpcClient,
    identity_access::identity_access_client::IdentityAccessClient as IdentityAccessGrpcClient,
    realtime::realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient,
    runtime::runtime_probe_client::RuntimeProbeClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient,
};
use std::time::Duration;
use tokio::{net::TcpListener, sync::oneshot};
use tonic::transport::Endpoint;

#[tokio::test]
async fn real_http_bff_classifies_grpc_dependency_timeout() {
    let (provider, capture_store) = http_grpc_acceptance_support::initialize_capture();
    let client = reqwest::Client::new();
    let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
    let traceparent = format!("00-{trace_id}-00f067aa0ba902b7-01");
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
    let blackhole_channel =
        tonic::transport::Endpoint::from_shared(format!("http://{blackhole_addr}"))
            .expect("blackhole endpoint parses")
            .connect_lazy();
    let timeout_identity_client = IdentityAccessGrpcClient::new(blackhole_channel.clone());
    let blackhole_tenant_client = TenantRegistryGrpcClient::new(blackhole_channel.clone());
    let blackhole_catalog_client = CatalogPricingGrpcClient::new(blackhole_channel.clone());
    let blackhole_entitlements_client = EntitlementsQuotasGrpcClient::new(blackhole_channel);
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
                blackhole_catalog_client,
                blackhole_entitlements_client,
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

    http_grpc_timeout_assertions::verify(&capture_store, timeout_correlation_id);

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
