use chrono::{DateTime, Utc};
use platform_contracts::{
    catalog_pricing::{GetPriceVersionRequest, catalog_pricing_client::CatalogPricingClient},
    entitlements_quotas::{
        AssignTenantPlanRequest, AssignTenantPlanResponse, CapabilityInput, ConsumeQuotaRequest,
        ConsumeQuotaResponse, GetTenantEntitlementRequest, ListTenantEntitlementsRequest,
        ListTenantEntitlementsResponse, SetPlanCapabilitiesRequest, SetPlanCapabilitiesResponse,
        TenantEntitlementResponse, entitlements_quotas_server::EntitlementsQuotas,
    },
    identity_access::{
        AuthorizeApiKeyOperationRequest, AuthorizeOperatorOperationRequest,
        AuthorizeTenantOperationRequest, identity_access_client::IdentityAccessClient,
    },
    tenant_registry::{GetTenantRequest, tenant_registry_client::TenantRegistryClient},
};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::{
    application::entitlement_service::{EntitlementService, EntitlementServiceError},
    domain::entitlement::{Capability, Entitlement},
};

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
        parse_uuid(&response.operator_id)
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
        parse_uuid(&response.user_id)
    }
}

#[tonic::async_trait]
impl EntitlementsQuotas for EntitlementsQuotasApi {
    async fn set_plan_capabilities(
        &self,
        request: Request<SetPlanCapabilitiesRequest>,
    ) -> Result<Response<SetPlanCapabilitiesResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let actor = self
            .authorize_operator(body.session_token, body.csrf_token, body.correlation_id)
            .await?;
        let price_id = parse_uuid(&body.price_version_id)?;
        self.catalog
            .clone()
            .get_price_version(Request::new(GetPriceVersionRequest {
                price_version_id: price_id.to_string(),
            }))
            .await?;
        let capabilities = body
            .capabilities
            .into_iter()
            .map(map_capability)
            .collect::<Result<Vec<_>, _>>()?;
        let saved = self
            .service
            .set_capabilities(actor, price_id, &capabilities, &correlation_id)
            .await
            .map_err(map_error)?;
        Ok(Response::new(SetPlanCapabilitiesResponse {
            capabilities_saved: saved,
        }))
    }

    async fn assign_tenant_plan(
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
            )
            .await?;
        let tenant_id = parse_uuid(&body.tenant_id)?;
        let tenant = self
            .tenants
            .clone()
            .get_tenant(Request::new(GetTenantRequest {
                session_token: body.session_token,
                tenant_id: tenant_id.to_string(),
                correlation_id: body.correlation_id.clone(),
                csrf_token: body.csrf_token,
            }))
            .await?
            .into_inner();
        if tenant.status != "active" {
            return Err(Status::failed_precondition("tenant is not active"));
        }
        let price_id = parse_uuid(&body.price_version_id)?;
        let price = self
            .catalog
            .clone()
            .get_price_version(Request::new(GetPriceVersionRequest {
                price_version_id: price_id.to_string(),
            }))
            .await?
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
        let assignment = self
            .service
            .assign_plan(actor, tenant_id, price_id, from, until, &correlation_id)
            .await
            .map_err(map_error)?;
        Ok(Response::new(AssignTenantPlanResponse {
            assignment_id: assignment.to_string(),
        }))
    }

    async fn get_tenant_entitlement(
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

    async fn list_tenant_entitlements(
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

    async fn consume_quota(
        &self,
        request: Request<ConsumeQuotaRequest>,
    ) -> Result<Response<ConsumeQuotaResponse>, Status> {
        let body = request.into_inner();
        let tenant = parse_uuid(&body.tenant_id)?;
        let correlation_id = body.correlation_id.clone();
        let authorization = self
            .identity
            .clone()
            .authorize_api_key_operation(Request::new(AuthorizeApiKeyOperationRequest {
                secret: body.api_key_secret,
                tenant_id: tenant.to_string(),
                environment: body.environment.clone(),
                required_scope: "usage.consume".to_owned(),
                correlation_id: correlation_id.clone(),
            }))
            .await?
            .into_inner();
        if authorization.tenant_id != tenant.to_string()
            || authorization.environment != body.environment
        {
            return Err(Status::permission_denied(
                "API key is not authorized for this tenant environment",
            ));
        }
        let actor = parse_uuid(&authorization.key_id)?;
        let (entitlement, replayed) = self
            .service
            .consume(
                actor,
                tenant,
                &body.capability_key,
                &body.environment,
                body.quantity,
                &body.idempotency_key,
                Utc::now(),
                &correlation_id,
            )
            .await
            .map_err(map_error)?;
        Ok(Response::new(ConsumeQuotaResponse {
            entitlement: Some(entitlement_response(entitlement)),
            replayed,
        }))
    }
}

fn map_capability(value: CapabilityInput) -> Result<Capability, Status> {
    let capability = Capability {
        key: value.capability_key,
        enabled: value.enabled,
        limit_value: value.limit_value,
        unit: value.unit,
        overage_policy: value.overage_policy,
        environment: value.environment,
        quota_period: value.quota_period,
    };
    capability
        .validate()
        .map_err(|_| Status::invalid_argument("capability input is invalid"))?;
    Ok(capability)
}

fn entitlement_response(value: Entitlement) -> TenantEntitlementResponse {
    TenantEntitlementResponse {
        tenant_id: value.tenant_id.to_string(),
        price_version_id: value.price_version_id.to_string(),
        capability_key: value.capability_key,
        enabled: value.enabled,
        limit_value: value.limit_value,
        consumed: value.consumed,
        remaining: value.remaining,
        allowed: value.allowed,
        reason: value.reason,
        environment: value.environment,
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("identifier is invalid"))
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, Status> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| Status::invalid_argument("timestamp is invalid"))
}

fn map_error(error: EntitlementServiceError) -> Status {
    match error {
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::InvalidCapability,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::InvalidQuantity,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::InvalidIdempotencyKey,
        ) => Status::invalid_argument("entitlement input is invalid"),
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::CapabilityUnavailable,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::PlanNotAssigned,
        ) => Status::not_found("entitlement was not found"),
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::QuotaExceeded,
        ) => Status::resource_exhausted("quota is exhausted"),
        EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::IdempotencyConflict,
        )
        | EntitlementServiceError::Domain(
            crate::domain::entitlement::EntitlementError::AssignmentConflict,
        ) => Status::already_exists("entitlement operation conflicts"),
        EntitlementServiceError::Database(error) => {
            tracing::error!(event.name="entitlements.persistence.failed", component="entitlements_quotas", error_class=%error);
            Status::internal("entitlement operation could not be completed")
        }
    }
}
