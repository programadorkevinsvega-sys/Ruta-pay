use platform_contracts::{
    catalog_pricing::GetPriceVersionRequest,
    entitlements_quotas::{
        AssignTenantPlanRequest, AssignTenantPlanResponse, SetPlanCapabilitiesRequest,
        SetPlanCapabilitiesResponse,
    },
    tenant_registry::GetTenantRequest,
};
use tonic::{Request, Response, Status};

use super::{
    EntitlementsQuotasApi,
    context::contextual_request,
    diagnostics::dependency_error,
    mapping::{map_capability, map_error, parse_time, parse_uuid},
};

impl EntitlementsQuotasApi {
    pub(super) async fn set_plan_capabilities_handler(
        &self,
        request: Request<SetPlanCapabilitiesRequest>,
    ) -> Result<Response<SetPlanCapabilitiesResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let actor = self
            .authorize_operator(
                body.session_token,
                body.csrf_token,
                correlation_id.clone(),
                "set_plan_capabilities",
            )
            .await?;
        let price_id = parse_uuid(&body.price_version_id)?;
        let started = std::time::Instant::now();
        let price_request = contextual_request(
            GetPriceVersionRequest {
                price_version_id: price_id.to_string(),
            },
            &correlation_id,
        )?;
        self.catalog
            .clone()
            .get_price_version(price_request)
            .await
            .map_err(|error| {
                dependency_error(
                    error,
                    "set_plan_capabilities",
                    "catalog_pricing",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )
            })?;
        let capabilities = body
            .capabilities
            .into_iter()
            .map(map_capability)
            .collect::<Result<Vec<_>, _>>()?;
        let started = std::time::Instant::now();
        let saved = self
            .service
            .set_capabilities(actor, price_id, &capabilities, &correlation_id)
            .await
            .map_err(|error| {
                map_error(
                    error,
                    "set_plan_capabilities",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )
            })?;
        Ok(Response::new(SetPlanCapabilitiesResponse {
            capabilities_saved: saved,
        }))
    }
    pub(super) async fn assign_tenant_plan_handler(
        &self,
        request: Request<AssignTenantPlanRequest>,
    ) -> Result<Response<AssignTenantPlanResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let actor = self
            .authorize_operator(
                body.session_token.clone(),
                body.csrf_token.clone(),
                body.correlation_id.clone(),
                "assign_tenant_plan",
            )
            .await?;
        let tenant_id = parse_uuid(&body.tenant_id)?;
        let started = std::time::Instant::now();
        let tenant_request = contextual_request(
            GetTenantRequest {
                session_token: body.session_token.clone(),
                tenant_id: tenant_id.to_string(),
                correlation_id: correlation_id.clone(),
                csrf_token: body.csrf_token.clone(),
            },
            &correlation_id,
        )?;
        let tenant = self
            .tenants
            .clone()
            .get_tenant(tenant_request)
            .await
            .map_err(|error| {
                dependency_error(
                    error,
                    "assign_tenant_plan",
                    "tenant_registry",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )
            })?
            .into_inner();
        if tenant.status != "active" {
            return Err(Status::failed_precondition("tenant is not active"));
        }
        let price_id = parse_uuid(&body.price_version_id)?;
        let started = std::time::Instant::now();
        let price_request = contextual_request(
            GetPriceVersionRequest {
                price_version_id: price_id.to_string(),
            },
            &correlation_id,
        )?;
        let price = self
            .catalog
            .clone()
            .get_price_version(price_request)
            .await
            .map_err(|error| {
                dependency_error(
                    error,
                    "assign_tenant_plan",
                    "catalog_pricing",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )
            })?
            .into_inner();
        let from = parse_time(&body.effective_from)?;
        let until = if body.effective_until.is_empty() {
            None
        } else {
            Some(parse_time(&body.effective_until)?)
        };
        let price_from = parse_time(&price.effective_from)?;
        let price_until = if price.effective_until.is_empty() {
            None
        } else {
            Some(parse_time(&price.effective_until)?)
        };
        let exceeds_price_end = match (until, price_until) {
            (None, Some(_)) => true,
            (Some(assignment_end), Some(price_end)) => assignment_end > price_end,
            _ => false,
        };
        if from < price_from || price_until.is_some_and(|end| from >= end) || exceeds_price_end {
            return Err(Status::invalid_argument(
                "assignment period is outside price validity",
            ));
        }
        let started = std::time::Instant::now();
        let assignment = self
            .service
            .assign_plan(actor, tenant_id, price_id, from, until, &correlation_id)
            .await
            .map_err(|error| {
                map_error(
                    error,
                    "assign_tenant_plan",
                    &correlation_id,
                    started.elapsed().as_millis() as u64,
                )
            })?;
        Ok(Response::new(AssignTenantPlanResponse {
            assignment_id: assignment.to_string(),
        }))
    }
}
