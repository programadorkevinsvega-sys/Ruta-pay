mod handlers;
mod mapping;

use platform_contracts::identity_access::{
    AuthorizeOperatorOperationRequest, identity_access_client::IdentityAccessClient,
};
use tonic::{Request, Status};
use uuid::Uuid;

use crate::application::catalog_service::CatalogService;

#[derive(Clone)]
pub struct CatalogPricingApi {
    service: CatalogService,
    identity: IdentityAccessClient<tonic::transport::Channel>,
}

impl CatalogPricingApi {
    pub fn new(
        service: CatalogService,
        identity: IdentityAccessClient<tonic::transport::Channel>,
    ) -> Self {
        Self { service, identity }
    }

    async fn authorize_operator(
        &self,
        token: String,
        csrf: String,
        correlation: String,
    ) -> Result<Uuid, Status> {
        let response = self
            .identity
            .clone()
            .authorize_operator_operation(Request::new(AuthorizeOperatorOperationRequest {
                session_token: token,
                csrf_token: csrf,
                permission: "catalog.manage".to_owned(),
                correlation_id: correlation,
            }))
            .await?
            .into_inner();
        Uuid::parse_str(&response.operator_id)
            .map_err(|_| Status::internal("operator identity is invalid"))
    }
}
