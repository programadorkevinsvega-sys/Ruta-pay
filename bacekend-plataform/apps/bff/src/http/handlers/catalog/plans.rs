use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::http::{handlers::identity, middleware::correlation_id::correlation_id, routes::AppState};
use platform_contracts::catalog_pricing::{CreatePlanRequest, ListPlansRequest};
use super::{shared::{operator_credentials, tonic_request}, types::{NewPlan, PlanView}};

pub async fn create_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NewPlan>,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some((session, csrf)) = operator_credentials(&headers) else {
        return identity::error(
            StatusCode::FORBIDDEN,
            "csrf_validation_failed",
            "authorization",
            false,
            correlation,
        );
    };
    let request = CreatePlanRequest {
        session_token: session,
        csrf_token: csrf,
        slug: body.slug,
        name: body.name,
        description: body.description,
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match state
        .catalog_client
        .clone()
        .create_plan(tonic_request(request, &correlation))
        .await
    {
        Ok(response) => {
            let p = response.into_inner();
            (
                StatusCode::CREATED,
                Json(PlanView {
                    plan_id: p.plan_id,
                    slug: p.slug,
                    name: p.name,
                    description: p.description,
                    status: p.status,
                    created_at: p.created_at,
                }),
            )
                .into_response()
        }
        Err(status) => identity::grpc_error(
            status,
            "create_plan",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path="/v1/platform/catalog/plans", responses((status=200, body=Vec<PlanView>)))]

pub async fn list_plans(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let correlation = correlation_id(&headers);
    let Some((session, csrf)) = operator_credentials(&headers) else {
        return identity::error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let request = ListPlansRequest {
        session_token: session,
        csrf_token: csrf,
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match state
        .catalog_client
        .clone()
        .list_plans(tonic_request(request, &correlation))
        .await
    {
        Ok(response) => Json(
            response
                .into_inner()
                .plans
                .into_iter()
                .map(|p| PlanView {
                    plan_id: p.plan_id,
                    slug: p.slug,
                    name: p.name,
                    description: p.description,
                    status: p.status,
                    created_at: p.created_at,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(status) => identity::grpc_error(
            status,
            "list_plans",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(post, path="/v1/platform/catalog/plans/{plan_id}/prices", request_body=NewPriceVersion, responses((status=201, body=PriceView)))]
