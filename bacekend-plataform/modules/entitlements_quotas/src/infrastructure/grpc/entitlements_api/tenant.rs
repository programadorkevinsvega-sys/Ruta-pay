use chrono::Utc;
use platform_contracts::entitlements_quotas::{
    GetTenantEntitlementRequest, ListTenantEntitlementsRequest, ListTenantEntitlementsResponse,
    TenantEntitlementResponse,
};
use tonic::{Request, Response, Status};

use super::{
    EntitlementsQuotasApi,
    mapping::{entitlement_response, map_error, parse_time, parse_uuid},
};

impl EntitlementsQuotasApi {
    pub(super) async fn get_tenant_entitlement_handler(
        &self,
        request: Request<GetTenantEntitlementRequest>,
    ) -> Result<Response<TenantEntitlementResponse>, Status> {
        let body = request.into_inner();
        let tenant = parse_uuid(&body.tenant_id)?;
        let actor = self
            .authorize_tenant(
                body.session_token,
                body.csrf_token,
                tenant,
                "usage.read",
                body.correlation_id,
            )
            .await?;
        let at = if body.effective_at.is_empty() {
            Utc::now()
        } else {
            parse_time(&body.effective_at)?
        };
        let result = self
            .service
            .evaluate(actor, tenant, &body.capability_key, &body.environment, at)
            .await
            .map_err(map_error)?;
        Ok(Response::new(entitlement_response(result)))
    }
    pub(super) async fn list_tenant_entitlements_handler(
        &self,
        request: Request<ListTenantEntitlementsRequest>,
    ) -> Result<Response<ListTenantEntitlementsResponse>, Status> {
        let body = request.into_inner();
        let tenant = parse_uuid(&body.tenant_id)?;
        let actor = self
            .authorize_tenant(
                body.session_token,
                body.csrf_token,
                tenant,
                "usage.read",
                body.correlation_id,
            )
            .await?;
        let at = if body.effective_at.is_empty() {
            Utc::now()
        } else {
            parse_time(&body.effective_at)?
        };
        let entitlements = self
            .service
            .list(actor, tenant, &body.environment, at)
            .await
            .map_err(map_error)?;
        Ok(Response::new(ListTenantEntitlementsResponse {
            entitlements: entitlements.into_iter().map(entitlement_response).collect(),
        }))
    }
}
