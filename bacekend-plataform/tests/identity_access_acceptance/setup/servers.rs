use std::net::{Ipv4Addr, SocketAddr};
use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient as CatalogPricingGrpcClient,
    entitlements_quotas::entitlements_quotas_client::EntitlementsQuotasClient as EntitlementsQuotasGrpcClient,
    identity_access::identity_access_client::IdentityAccessClient,
    realtime::realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient,
    runtime::runtime_probe_client::RuntimeProbeClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient,
};
use reqwest::{Client, header};
use tokio::{net::TcpListener, sync::oneshot};
use uuid::Uuid;

pub(super) struct ServerState {
    pub(super) grpc_addr: SocketAddr,
    pub(super) grpc_task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    pub(super) grpc_stop: oneshot::Sender<()>,
    pub(super) authorization_client: IdentityAccessClient<tonic::transport::Channel>,
    pub(super) realtime_acceptance_client: RealtimeGatewayGrpcClient<tonic::transport::Channel>,
    pub(super) http_addr: SocketAddr,
    pub(super) origin: String,
    pub(super) http_task: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    pub(super) http_stop: oneshot::Sender<()>,
    pub(super) default_headers: header::HeaderMap,
    pub(super) test_source_ip: Ipv4Addr,
    pub(super) client: Client,
    pub(super) email: String,
    pub(super) password: String,
}

pub(super) async fn start(
    runtime_pool: sqlx::PgPool,
    redis_client: redis::Client,
    redis_manager: redis::aio::ConnectionManager,
) -> ServerState {
let grpc_listener = TcpListener::bind("127.0.0.1:0").await.expect("gRPC bind");
let grpc_addr = grpc_listener.local_addr().expect("gRPC address");
let runtime_channel = tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
    .expect("identity module endpoint parses")
    .connect_lazy();
let module_identity_client = IdentityAccessClient::new(runtime_channel.clone());
let module_catalog_client = CatalogPricingGrpcClient::new(runtime_channel.clone());
let module_tenant_client = TenantRegistryGrpcClient::new(runtime_channel);
let (grpc_stop, grpc_signal) = oneshot::channel();
let grpc_pool = runtime_pool.clone();
let grpc_task = tokio::spawn(platform_runtime::module_config::serve(
    grpc_listener,
    platform_runtime::module_config::RuntimeModuleDependencies {
        pool: grpc_pool,
        redis: redis_manager.clone(),
        redis_client,
        identity_client: module_identity_client,
        catalog_client: module_catalog_client,
        tenant_client: module_tenant_client,
    },
    async move {
        let _ = grpc_signal.await;
    },
));
let grpc_endpoint = tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
    .expect("gRPC endpoint parses");
let runtime_client = RuntimeProbeClient::connect(grpc_endpoint.clone())
    .await
    .expect("runtime gRPC channel connects");
let identity_client = IdentityAccessClient::connect(grpc_endpoint)
    .await
    .expect("identity gRPC channel connects");
let tenant_client = TenantRegistryGrpcClient::connect(format!("http://{grpc_addr}"))
    .await
    .expect("tenant registry gRPC channel connects");
let realtime_client = RealtimeGatewayGrpcClient::connect(format!("http://{grpc_addr}"))
    .await
    .expect("realtime gRPC channel connects");
let catalog_client = CatalogPricingGrpcClient::connect(format!("http://{grpc_addr}"))
    .await
    .expect("catalog gRPC channel connects");
let entitlements_client = EntitlementsQuotasGrpcClient::connect(format!("http://{grpc_addr}"))
    .await
    .expect("entitlements gRPC channel connects");
let mut authorization_client = identity_client.clone();
let realtime_acceptance_client = realtime_client.clone();
let http_listener = TcpListener::bind("127.0.0.1:0").await.expect("BFF bind");
let http_addr = http_listener.local_addr().expect("BFF address");
let origin = format!("http://localhost:{}", http_addr.port());
let (http_stop, http_signal) = oneshot::channel();
let http_task = tokio::spawn(async move {
    axum::serve(
        http_listener,
        payvia_platform_bff::http::routes::router(
            runtime_client,
            identity_client,
            tenant_client,
            realtime_client,
            catalog_client,
            entitlements_client,
        )
        .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = http_signal.await;
    })
    .await
});
let mut default_headers = header::HeaderMap::new();
default_headers.insert(
    header::HOST,
    format!("localhost:{}", http_addr.port())
        .parse()
        .expect("local host header is valid"),
);
let source_id = Uuid::new_v4();
let source_bytes = source_id.as_bytes();
let test_source_ip = std::net::Ipv4Addr::new(
    127,
    source_bytes[0],
    source_bytes[1],
    source_bytes[2].max(1),
);
let client = Client::builder()
    .local_address(std::net::IpAddr::V4(test_source_ip))
    .default_headers(default_headers.clone())
    .build()
    .expect("HTTP acceptance client builds");
let email = format!("acceptance_{}@example.test", Uuid::new_v4().simple());
let password = format!("A-valid-password-{}!", Uuid::new_v4().simple());
    ServerState {
        grpc_addr,
        grpc_task,
        grpc_stop,
        authorization_client,
        realtime_acceptance_client,
        http_addr,
        origin,
        http_task,
        http_stop,
        default_headers,
        test_source_ip,
        client,
        email,
        password,
    }
}
