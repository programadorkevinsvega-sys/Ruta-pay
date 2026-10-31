use chrono::{DateTime, Utc};
use platform_contracts::{
    catalog_pricing::{
        catalog_pricing_server::CatalogPricing, CreatePlanRequest, GetPriceVersionRequest,
        ListPlansRequest, ListPlansResponse, PlanResponse, PriceVersionResponse,
        PublishPriceVersionRequest,
    },
    identity_access::{identity_access_client::IdentityAccessClient, AuthorizeOperatorOperationRequest},
};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::application::catalog_service::{CatalogService, CatalogServiceError};

#[derive(Clone)]
pub struct CatalogPricingApi {
    service: CatalogService,
    identity: IdentityAccessClient<tonic::transport::Channel>,
}

impl CatalogPricingApi {
    pub fn new(service: CatalogService, identity: IdentityAccessClient<tonic::transport::Channel>) -> Self {
        Self { service, identity }
    }

    async fn authorize_operator(&self, token: String, csrf: String, correlation: String) -> Result<Uuid, Status> {
        let response = self.identity.clone().authorize_operator_operation(Request::new(
            AuthorizeOperatorOperationRequest {
                session_token: token,
                csrf_token: csrf,
                permission: "catalog.manage".to_owned(),
                correlation_id: correlation,
            },
        )).await?.into_inner();
        Uuid::parse_str(&response.operator_id)
            .map_err(|_| Status::internal("operator identity is invalid"))
    }
}

#[tonic::async_trait]
impl CatalogPricing for CatalogPricingApi {
    async fn create_plan(&self, request: Request<CreatePlanRequest>) -> Result<Response<PlanResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let actor = self.authorize_operator(body.session_token, body.csrf_token, body.correlation_id).await?;
        let plan = self.service.create_plan(actor, &body.slug, &body.name, &body.description, &correlation_id)
            .await.map_err(map_error)?;
        Ok(Response::new(PlanResponse {
            plan_id: plan.id.to_string(), slug: plan.slug, name: plan.name,
            description: plan.description, status: plan.status, created_at: plan.created_at.to_rfc3339(),
        }))
    }

    async fn publish_price_version(&self, request: Request<PublishPriceVersionRequest>) -> Result<Response<PriceVersionResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let actor = self.authorize_operator(body.session_token, body.csrf_token, body.correlation_id).await?;
        let plan_id = parse_uuid(&body.plan_id)?;
        let from = parse_time(&body.effective_from)?;
        let until = if body.effective_until.is_empty() { None } else { Some(parse_time(&body.effective_until)?) };
        let interval = i32::try_from(body.interval_count).map_err(|_| Status::invalid_argument("interval_count is invalid"))?;
        let price = self.service.publish_price_version(actor, plan_id, &body.currency, body.amount_minor,
            &body.period_unit, interval, from, until, &correlation_id).await.map_err(map_error)?;
        Ok(Response::new(price_response(price)))
    }

    async fn get_price_version(&self, request: Request<GetPriceVersionRequest>) -> Result<Response<PriceVersionResponse>, Status> {
        let id = parse_uuid(&request.into_inner().price_version_id)?;
        let price = self.service.get_price_version(id).await.map_err(map_error)?;
        Ok(Response::new(price_response(price)))
    }

    async fn list_plans(&self, request: Request<ListPlansRequest>) -> Result<Response<ListPlansResponse>, Status> {
        let body = request.into_inner();
        let actor = self.authorize_operator(body.session_token, body.csrf_token, body.correlation_id).await?;
        let plans = self.service.list_plans(actor).await.map_err(map_error)?;
        Ok(Response::new(ListPlansResponse {
            plans: plans.into_iter().map(|plan| PlanResponse {
                plan_id: plan.id.to_string(), slug: plan.slug, name: plan.name,
                description: plan.description, status: plan.status, created_at: plan.created_at.to_rfc3339(),
            }).collect(),
        }))
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("identifier is invalid"))
}
fn parse_time(value: &str) -> Result<DateTime<Utc>, Status> {
    DateTime::parse_from_rfc3339(value).map(|v| v.with_timezone(&Utc))
        .map_err(|_| Status::invalid_argument("timestamp is invalid"))
}
fn price_response(price: crate::domain::catalog::PriceVersion) -> PriceVersionResponse {
    PriceVersionResponse {
        price_version_id: price.id.to_string(), plan_id: price.plan_id.to_string(),
        version: u32::try_from(price.version).unwrap_or_default(), currency: price.currency,
        amount_minor: price.amount_minor, period_unit: price.period_unit,
        interval_count: u32::try_from(price.interval_count).unwrap_or_default(),
        effective_from: price.effective_from.to_rfc3339(),
        effective_until: price.effective_until.map(|v| v.to_rfc3339()).unwrap_or_default(),
        published_at: price.published_at.to_rfc3339(),
    }
}
fn map_error(error: CatalogServiceError) -> Status {
    match error {
        CatalogServiceError::Domain(crate::domain::catalog::CatalogError::InvalidPlan)
        | CatalogServiceError::Domain(crate::domain::catalog::CatalogError::InvalidPrice) =>
            Status::invalid_argument("catalog input is invalid"),
        CatalogServiceError::Domain(crate::domain::catalog::CatalogError::PlanNotFound)
        | CatalogServiceError::Domain(crate::domain::catalog::CatalogError::PriceVersionNotFound) =>
            Status::not_found("catalog resource was not found"),
        CatalogServiceError::Domain(crate::domain::catalog::CatalogError::EffectivePeriodConflict) =>
            Status::already_exists("price effective period conflicts"),
        CatalogServiceError::Database(error) => {
            tracing::error!(event.name="catalog.persistence.failed", component="catalog_pricing", error_class=%error);
            Status::internal("catalog operation could not be completed")
        }
    }
}
