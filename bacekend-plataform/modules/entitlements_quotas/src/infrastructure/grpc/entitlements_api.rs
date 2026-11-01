mod admin;
mod consume;
mod mapping;
mod tenant;

use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient,
    entitlements_quotas::{
        AssignTenantPlanRequest, AssignTenantPlanResponse, ConsumeQuotaRequest,
        ConsumeQuotaResponse, GetTenantEntitlementRequest, ListTenantEntitlementsRequest,
        ListTenantEntitlementsResponse, SetPlanCapabilitiesRequest, SetPlanCapabilitiesResponse,
        TenantEntitlementResponse, entitlements_quotas_server::EntitlementsQuotas,
    },
    identity_access::{
        AuthorizeOperatorOperationRequest, AuthorizeTenantOperationRequest,
        identity_access_client::IdentityAccessClient,
    },
    tenant_registry::tenant_registry_client::TenantRegistryClient,
};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::application::entitlement_service::EntitlementService;

#[derive(Clone)]
pub struct EntitlementsQuotasApi {
    service: EntitlementService,
    identity: IdentityAccessClient<tonic::transport::Channel>,
    catalog: CatalogPricingClient<tonic::transport::Channel>,
    tenants: TenantRegistryClient<tonic::transport::Channel>,
}

impl EntitlementsQuotasApi {
    pub fn new(
        service: EntitlementService,
        identity: IdentityAccessClient<tonic::transport::Channel>,
        catalog: CatalogPricingClient<tonic::transport::Channel>,
        tenants: TenantRegistryClient<tonic::transport::Channel>,
    ) -> Self {
        Self {
            service,
            identity,
            catalog,
            tenants,
        }
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

    async fn authorize_tenant(
        &self,
        token: String,
        csrf: String,
        tenant: Uuid,
        permission: &str,
        correlation: String,
    ) -> Result<Uuid, Status> {
        let response = self
            .identity
            .clone()
            .authorize_tenant_operation(Request::new(AuthorizeTenantOperationRequest {
                session_token: token,
                csrf_token: csrf,
                tenant_id: tenant.to_string(),
                permission: permission.to_owned(),
                correlation_id: correlation,
            }))
            .await?
            .into_inner();
        Uuid::parse_str(&response.user_id)
            .map_err(|_| Status::internal("tenant identity is invalid"))
    }
}

#[tonic::async_trait]
impl EntitlementsQuotas for EntitlementsQuotasApi {
    async fn set_plan_capabilities(
        &self,
        request: Request<SetPlanCapabilitiesRequest>,
    ) -> Result<Response<SetPlanCapabilitiesResponse>, Status> {
        self.set_plan_capabilities_handler(request).await
    }

    async fn assign_tenant_plan(
        &self,
        request: Request<AssignTenantPlanRequest>,
    ) -> Result<Response<AssignTenantPlanResponse>, Status> {
        self.assign_tenant_plan_handler(request).await
    }

    async fn get_tenant_entitlement(
        &self,
        request: Request<GetTenantEntitlementRequest>,
    ) -> Result<Response<TenantEntitlementResponse>, Status> {
        self.get_tenant_entitlement_handler(request).await
    }

    async fn list_tenant_entitlements(
        &self,
        request: Request<ListTenantEntitlementsRequest>,
    ) -> Result<Response<ListTenantEntitlementsResponse>, Status> {
        self.list_tenant_entitlements_handler(request).await
    }

    async fn consume_quota(
        &self,
        request: Request<ConsumeQuotaRequest>,
    ) -> Result<Response<ConsumeQuotaResponse>, Status> {
        self.consume_quota_handler(request).await
    }
}
