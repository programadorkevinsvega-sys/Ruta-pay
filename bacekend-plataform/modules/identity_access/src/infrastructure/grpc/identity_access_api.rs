use crate::application::identity_service::{IdentityError, IdentityService};
use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
use payvia_observability::{current_span_id, current_trace_id};
use platform_contracts::identity_access::{
    ApiKeyMetadata, AuthorizeApiKeyOperationRequest, AuthorizeApiKeyOperationResponse,
    AuthorizeOperatorOperationRequest, AuthorizeOperatorOperationResponse,
    AuthorizeTenantOperationRequest, AuthorizeTenantOperationResponse, CreateApiKeyRequest,
    CreateApiKeyResponse, CreateOperatorSessionRequest, CreateOperatorSessionResponse,
    CreateTenantSessionRequest, CreateTenantSessionResponse, ListApiKeysRequest,
    ListApiKeysResponse, ListTenantMembershipsRequest, ListTenantMembershipsResponse,
    RegisterTenantUserRequest, RegisterTenantUserResponse, RevokeApiKeyRequest,
    RevokeApiKeyResponse, RevokeOperatorSessionRequest, RevokeOperatorSessionResponse,
    RevokeSessionRequest, RevokeSessionResponse, TenantMembership, ValidateOperatorSessionRequest,
    ValidateOperatorSessionResponse, ValidateTenantSessionRequest, ValidateTenantSessionResponse,
    identity_access_server::IdentityAccess,
};
use tonic::{Request, Response, Status};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

mod api_keys;
mod operator;
mod support;
mod tenant;

#[derive(Clone)]
pub struct IdentityAccessApi {
    service: IdentityService,
}

impl IdentityAccessApi {
    pub fn new(service: IdentityService) -> Self {
        Self { service }
    }
}

#[tonic::async_trait]
impl IdentityAccess for IdentityAccessApi {
    async fn register_tenant_user(
        &self,
        request: Request<RegisterTenantUserRequest>,
    ) -> Result<Response<RegisterTenantUserResponse>, Status> {
        IdentityAccessApi::register_tenant_user(self, request).await
    }

    async fn create_tenant_session(
        &self,
        request: Request<CreateTenantSessionRequest>,
    ) -> Result<Response<CreateTenantSessionResponse>, Status> {
        IdentityAccessApi::create_tenant_session(self, request).await
    }

    async fn revoke_session(
        &self,
        request: Request<RevokeSessionRequest>,
    ) -> Result<Response<RevokeSessionResponse>, Status> {
        IdentityAccessApi::revoke_session(self, request).await
    }

    async fn validate_tenant_session(
        &self,
        request: Request<ValidateTenantSessionRequest>,
    ) -> Result<Response<ValidateTenantSessionResponse>, Status> {
        IdentityAccessApi::validate_tenant_session(self, request).await
    }

    async fn create_api_key(
        &self,
        request: Request<CreateApiKeyRequest>,
    ) -> Result<Response<CreateApiKeyResponse>, Status> {
        IdentityAccessApi::create_api_key(self, request).await
    }

    async fn list_api_keys(
        &self,
        request: Request<ListApiKeysRequest>,
    ) -> Result<Response<ListApiKeysResponse>, Status> {
        IdentityAccessApi::list_api_keys(self, request).await
    }

    async fn revoke_api_key(
        &self,
        request: Request<RevokeApiKeyRequest>,
    ) -> Result<Response<RevokeApiKeyResponse>, Status> {
        IdentityAccessApi::revoke_api_key(self, request).await
    }

    async fn authorize_api_key_operation(
        &self,
        request: Request<AuthorizeApiKeyOperationRequest>,
    ) -> Result<Response<AuthorizeApiKeyOperationResponse>, Status> {
        IdentityAccessApi::authorize_api_key_operation(self, request).await
    }

    async fn create_operator_session(
        &self,
        request: Request<CreateOperatorSessionRequest>,
    ) -> Result<Response<CreateOperatorSessionResponse>, Status> {
        IdentityAccessApi::create_operator_session(self, request).await
    }

    async fn revoke_operator_session(
        &self,
        request: Request<RevokeOperatorSessionRequest>,
    ) -> Result<Response<RevokeOperatorSessionResponse>, Status> {
        IdentityAccessApi::revoke_operator_session(self, request).await
    }

    async fn validate_operator_session(
        &self,
        request: Request<ValidateOperatorSessionRequest>,
    ) -> Result<Response<ValidateOperatorSessionResponse>, Status> {
        IdentityAccessApi::validate_operator_session(self, request).await
    }

    async fn authorize_operator_operation(
        &self,
        request: Request<AuthorizeOperatorOperationRequest>,
    ) -> Result<Response<AuthorizeOperatorOperationResponse>, Status> {
        IdentityAccessApi::authorize_operator_operation(self, request).await
    }

    async fn authorize_tenant_operation(
        &self,
        request: Request<AuthorizeTenantOperationRequest>,
    ) -> Result<Response<AuthorizeTenantOperationResponse>, Status> {
        IdentityAccessApi::authorize_tenant_operation(self, request).await
    }

    async fn list_tenant_memberships(
        &self,
        request: Request<ListTenantMembershipsRequest>,
    ) -> Result<Response<ListTenantMembershipsResponse>, Status> {
        IdentityAccessApi::list_tenant_memberships(self, request).await
    }
}
