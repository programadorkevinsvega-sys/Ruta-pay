use platform_contracts::catalog_pricing::{
    CreatePlanRequest, GetPriceVersionRequest, ListPlansRequest, ListPlansResponse, PlanResponse,
    PriceVersionResponse, PublishPriceVersionRequest, catalog_pricing_server::CatalogPricing,
};
use tonic::{Request, Response, Status};

use super::{
    CatalogPricingApi,
    mapping::{map_error, parse_time, parse_uuid, price_response},
};
use crate::application::catalog_service::PublishPriceVersionCommand;

impl CatalogPricing for CatalogPricingApi {
    async fn create_plan(
        &self,
        request: Request<CreatePlanRequest>,
    ) -> Result<Response<PlanResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let actor = self
            .authorize_operator(body.session_token, body.csrf_token, body.correlation_id)
            .await?;
        let plan = self
            .service
            .create_plan(
                actor,
                &body.slug,
                &body.name,
                &body.description,
                &correlation_id,
            )
            .await
            .map_err(map_error)?;
        Ok(Response::new(PlanResponse {
            plan_id: plan.id.to_string(),
            slug: plan.slug,
            name: plan.name,
            description: plan.description,
            status: plan.status,
            created_at: plan.created_at.to_rfc3339(),
        }))
    }

    async fn publish_price_version(
        &self,
        request: Request<PublishPriceVersionRequest>,
    ) -> Result<Response<PriceVersionResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let actor = self
            .authorize_operator(body.session_token, body.csrf_token, body.correlation_id)
            .await?;
        let plan_id = parse_uuid(&body.plan_id)?;
        let from = parse_time(&body.effective_from)?;
        let until = if body.effective_until.is_empty() {
            None
        } else {
            Some(parse_time(&body.effective_until)?)
        };
        let interval = i32::try_from(body.interval_count)
            .map_err(|_| Status::invalid_argument("interval_count is invalid"))?;
        let price = self
            .service
            .publish_price_version(PublishPriceVersionCommand {
                actor_id: actor,
                plan_id,
                currency: &body.currency,
                amount_minor: body.amount_minor,
                period_unit: &body.period_unit,
                interval_count: interval,
                effective_from: from,
                effective_until: until,
                correlation_id: &correlation_id,
            })
            .await
            .map_err(map_error)?;
        Ok(Response::new(price_response(price)))
    }

    async fn get_price_version(
        &self,
        request: Request<GetPriceVersionRequest>,
    ) -> Result<Response<PriceVersionResponse>, Status> {
        let id = parse_uuid(&request.into_inner().price_version_id)?;
        let price = self
            .service
            .get_price_version(id)
            .await
            .map_err(map_error)?;
        Ok(Response::new(price_response(price)))
    }

    async fn list_plans(
        &self,
        request: Request<ListPlansRequest>,
    ) -> Result<Response<ListPlansResponse>, Status> {
        let body = request.into_inner();
        let actor = self
            .authorize_operator(body.session_token, body.csrf_token, body.correlation_id)
            .await?;
        let plans = self.service.list_plans(actor).await.map_err(map_error)?;
        Ok(Response::new(ListPlansResponse {
            plans: plans
                .into_iter()
                .map(|plan| PlanResponse {
                    plan_id: plan.id.to_string(),
                    slug: plan.slug,
                    name: plan.name,
                    description: plan.description,
                    status: plan.status,
                    created_at: plan.created_at.to_rfc3339(),
                })
                .collect(),
        }))
    }
}
