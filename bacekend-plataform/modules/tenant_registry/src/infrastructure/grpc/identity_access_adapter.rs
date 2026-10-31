use async_trait::async_trait;
use platform_contracts::identity_access::{
    AuthorizeOperatorOperationRequest, AuthorizeTenantOperationRequest,
    ValidateTenantSessionRequest, identity_access_client::IdentityAccessClient as GrpcClient,
};
use tonic::{metadata::MetadataValue, transport::Channel};
use uuid::Uuid;

use crate::application::identity_access_port::{IdentityAccessError, IdentityAccessPort};

#[derive(Clone)]
pub struct IdentityAccessGrpcAdapter {
    client: GrpcClient<Channel>,
}

impl IdentityAccessGrpcAdapter {
    pub fn new(client: GrpcClient<Channel>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl IdentityAccessPort for IdentityAccessGrpcAdapter {
    async fn validate_tenant_session(
        &self,
        token: &str,
        csrf_token: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError> {
        let mut request = tonic::Request::new(ValidateTenantSessionRequest {
            session_token: token.to_owned(),
            csrf_token: csrf_token.to_owned(),
            correlation_id: correlation_id.to_owned(),
        });
        add_correlation(&mut request, correlation_id)?;
        let response = self
            .client
            .clone()
            .validate_tenant_session(request)
            .await
            .map_err(map_status)?
            .into_inner();
        Uuid::parse_str(&response.user_id).map_err(|_| IdentityAccessError::Unavailable)
    }

    async fn authorize_tenant(
        &self,
        token: &str,
        csrf_token: &str,
        tenant_id: Uuid,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError> {
        let mut request = tonic::Request::new(AuthorizeTenantOperationRequest {
            session_token: token.to_owned(),
            csrf_token: csrf_token.to_owned(),
            tenant_id: tenant_id.to_string(),
            permission: permission.to_owned(),
            correlation_id: correlation_id.to_owned(),
        });
        add_correlation(&mut request, correlation_id)?;
        let response = self
            .client
            .clone()
            .authorize_tenant_operation(request)
            .await
            .map_err(map_status)?
            .into_inner();
        Uuid::parse_str(&response.user_id).map_err(|_| IdentityAccessError::Unavailable)
    }

    async fn authorize_operator(
        &self,
        token: &str,
        csrf_token: &str,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError> {
        let mut request = tonic::Request::new(AuthorizeOperatorOperationRequest {
            session_token: token.to_owned(),
            csrf_token: csrf_token.to_owned(),
            permission: permission.to_owned(),
            correlation_id: correlation_id.to_owned(),
        });
        add_correlation(&mut request, correlation_id)?;
        let response = self
            .client
            .clone()
            .authorize_operator_operation(request)
            .await
            .map_err(map_status)?
            .into_inner();
        Uuid::parse_str(&response.operator_id).map_err(|_| IdentityAccessError::Unavailable)
    }
}

fn add_correlation<T>(
    request: &mut tonic::Request<T>,
    correlation_id: &str,
) -> Result<(), IdentityAccessError> {
    request.metadata_mut().insert(
        "x-correlation-id",
        MetadataValue::try_from(correlation_id).map_err(|_| IdentityAccessError::Invalid)?,
    );
    Ok(())
}

fn map_status(status: tonic::Status) -> IdentityAccessError {
    match status.code() {
        tonic::Code::Unauthenticated => IdentityAccessError::Authentication,
        tonic::Code::PermissionDenied => IdentityAccessError::Authorization,
        tonic::Code::InvalidArgument => IdentityAccessError::Invalid,
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
            IdentityAccessError::Unavailable
        }
        _ => IdentityAccessError::Unavailable,
    }
}
