use opentelemetry::propagation::Injector;
use platform_contracts::identity_access::{
    CreateApiKeyRequest, CreateApiKeyResponse, CreateTenantSessionRequest,
    CreateTenantSessionResponse, ListApiKeysRequest, ListApiKeysResponse,
    RegisterTenantUserRequest, RegisterTenantUserResponse, RevokeApiKeyRequest,
    RevokeApiKeyResponse, RevokeSessionRequest, RevokeSessionResponse,
    ValidateTenantSessionRequest, ValidateTenantSessionResponse,
    identity_access_client::IdentityAccessClient as GrpcClient,
};
use tonic::{
    metadata::{MetadataKey, MetadataMap, MetadataValue},
    transport::Channel,
};

pub struct IdentityAccessClient;

impl IdentityAccessClient {
    pub async fn register(
        client: &GrpcClient<Channel>,
        email: String,
        password: String,
        correlation_id: &str,
    ) -> Result<RegisterTenantUserResponse, tonic::Status> {
        let request = request_with_context(
            RegisterTenantUserRequest {
                email,
                password,
                correlation_id: correlation_id.to_owned(),
            },
            correlation_id,
        )?;
        let mut client = client.clone();
        client
            .register_tenant_user(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn login(
        client: &GrpcClient<Channel>,
        email: String,
        password: String,
        source_ip: String,
        correlation_id: &str,
    ) -> Result<CreateTenantSessionResponse, tonic::Status> {
        let request = request_with_context(
            CreateTenantSessionRequest {
                email,
                password,
                correlation_id: correlation_id.to_owned(),
                source_ip,
            },
            correlation_id,
        )?;
        let mut client = client.clone();
        client
            .create_tenant_session(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn logout(
        client: &GrpcClient<Channel>,
        token: String,
        correlation_id: &str,
    ) -> Result<RevokeSessionResponse, tonic::Status> {
        let request = request_with_context(
            RevokeSessionRequest {
                session_token: token,
                correlation_id: correlation_id.to_owned(),
            },
            correlation_id,
        )?;
        let mut client = client.clone();
        client
            .revoke_session(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn validate_session(
        client: &GrpcClient<Channel>,
        token: String,
        csrf_token: String,
        correlation_id: &str,
    ) -> Result<ValidateTenantSessionResponse, tonic::Status> {
        let request = request_with_context(
            ValidateTenantSessionRequest {
                session_token: token,
                csrf_token,
                correlation_id: correlation_id.to_owned(),
            },
            correlation_id,
        )?;
        let mut client = client.clone();
        client
            .validate_tenant_session(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn create_api_key(
        client: &GrpcClient<Channel>,
        body: CreateApiKeyRequest,
        correlation_id: &str,
    ) -> Result<CreateApiKeyResponse, tonic::Status> {
        let request = request_with_context(body, correlation_id)?;
        let mut client = client.clone();
        client
            .create_api_key(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn list_api_keys(
        client: &GrpcClient<Channel>,
        body: ListApiKeysRequest,
        correlation_id: &str,
    ) -> Result<ListApiKeysResponse, tonic::Status> {
        let request = request_with_context(body, correlation_id)?;
        let mut client = client.clone();
        client
            .list_api_keys(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn revoke_api_key(
        client: &GrpcClient<Channel>,
        body: RevokeApiKeyRequest,
        correlation_id: &str,
    ) -> Result<RevokeApiKeyResponse, tonic::Status> {
        let request = request_with_context(body, correlation_id)?;
        let mut client = client.clone();
        client
            .revoke_api_key(request)
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
