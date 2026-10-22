use opentelemetry::propagation::Injector;
use platform_contracts::tenant_registry::{
    ChangeTenantStatusRequest, GetTenantRequest, ListTenantsRequest, ListTenantsResponse,
    RegisterTenantRequest, TenantResponse,
    tenant_registry_client::TenantRegistryClient as GrpcClient,
};
use std::time::Duration;
use tonic::{
    metadata::{MetadataKey, MetadataMap, MetadataValue},
    transport::Channel,
};

pub struct TenantRegistryClient;

impl TenantRegistryClient {
    pub async fn register(
        client: &GrpcClient<Channel>,
        body: RegisterTenantRequest,
        correlation_id: &str,
    ) -> Result<TenantResponse, tonic::Status> {
        let request = request_with_context(body, correlation_id)?;
        let mut client = client.clone();
        client
            .register_tenant(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn change_status(
        client: &GrpcClient<Channel>,
        body: ChangeTenantStatusRequest,
        correlation_id: &str,
    ) -> Result<TenantResponse, tonic::Status> {
        let request = request_with_context(body, correlation_id)?;
        let mut client = client.clone();
        client
            .change_tenant_status(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn get(
        client: &GrpcClient<Channel>,
        body: GetTenantRequest,
        correlation_id: &str,
    ) -> Result<TenantResponse, tonic::Status> {
        let request = request_with_context(body, correlation_id)?;
        let mut client = client.clone();
        client
            .get_tenant(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn list(
        client: &GrpcClient<Channel>,
        body: ListTenantsRequest,
        correlation_id: &str,
    ) -> Result<ListTenantsResponse, tonic::Status> {
        let request = request_with_context(body, correlation_id)?;
        let mut client = client.clone();
        client
            .list_tenants(request)
            .await
            .map(tonic::Response::into_inner)
    }
}

fn request_with_context<T>(
    body: T,
    correlation_id: &str,
) -> Result<tonic::Request<T>, tonic::Status> {
    let mut request = tonic::Request::new(body);
    request.metadata_mut().insert(
        "x-correlation-id",
        MetadataValue::try_from(correlation_id)
            .map_err(|_| tonic::Status::invalid_argument("invalid correlation identifier"))?,
    );
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject(&mut MetadataInjector(request.metadata_mut()))
    });
    request.set_timeout(Duration::from_secs(3));
    Ok(request)
}

struct MetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(key), Ok(value)) = (
            MetadataKey::from_bytes(key.as_bytes()),
            MetadataValue::try_from(value),
        ) {
            self.0.insert(key, value);
        }
    }
}
