use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient as CatalogPricingGrpcClient,
    entitlements_quotas::entitlements_quotas_client::EntitlementsQuotasClient as EntitlementsQuotasGrpcClient,
    identity_access::identity_access_client::IdentityAccessClient as IdentityAccessGrpcClient,
    realtime::realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient,
    runtime::runtime_probe_client::RuntimeProbeClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

pub struct RuntimeClients {
    pub grpc: RuntimeProbeClient<tonic::transport::Channel>,
    pub identity: IdentityAccessGrpcClient<tonic::transport::Channel>,
    pub tenant: TenantRegistryGrpcClient<tonic::transport::Channel>,
    pub realtime: RealtimeGatewayGrpcClient<tonic::transport::Channel>,
    pub catalog: CatalogPricingGrpcClient<tonic::transport::Channel>,
    pub entitlements: EntitlementsQuotasGrpcClient<tonic::transport::Channel>,
}

pub struct RuntimeFixture {
    pub shutdown: oneshot::Sender<()>,
    pub task: JoinHandle<Result<(), tonic::transport::Error>>,
    pub clients: RuntimeClients,
}

pub async fn start_runtime() -> RuntimeFixture {
    let runtime_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("runtime bind");
    let runtime_addr = runtime_listener.local_addr().expect("runtime address");
    let runtime_channel = tonic::transport::Endpoint::from_shared(format!("http://{runtime_addr}"))
        .expect("identity module endpoint parses")
        .connect_lazy();
    let module_identity_client = IdentityAccessGrpcClient::new(runtime_channel.clone());
    let module_catalog_client = CatalogPricingGrpcClient::new(runtime_channel.clone());
    let module_tenant_client = TenantRegistryGrpcClient::new(runtime_channel);
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
        platform_runtime::module_config::RuntimeModuleDependencies {
            pool: database_pool,
            redis: redis_manager,
            redis_client,
            identity_client: module_identity_client,
            catalog_client: module_catalog_client,
            tenant_client: module_tenant_client,
        },
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
    let catalog_client = CatalogPricingGrpcClient::connect(format!("http://{runtime_addr}"))
        .await
        .expect("catalog gRPC connection");
    let entitlements_client =
        EntitlementsQuotasGrpcClient::connect(format!("http://{runtime_addr}"))
            .await
            .expect("entitlements gRPC connection");
    RuntimeFixture {
        shutdown: runtime_shutdown,
        task: runtime_task,
        clients: RuntimeClients {
            grpc: grpc_client,
            identity: identity_client,
            tenant: tenant_client,
            realtime: realtime_client,
            catalog: catalog_client,
            entitlements: entitlements_client,
        },
    }
}
