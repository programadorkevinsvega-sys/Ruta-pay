use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::http::{handlers::identity, middleware::correlation_id::correlation_id, routes::AppState};
use platform_contracts::catalog_pricing::PublishPriceVersionRequest;
use super::{shared::{operator_credentials, tonic_request}, types::{NewPriceVersion, PriceView}};

#[utoipa::path(post, path="/v1/platform/catalog/plans/{plan_id}/prices", request_body=NewPriceVersion, responses((status=201, body=PriceView)))]
pub async fn publish_price(
    State(state): State<Arc<AppState>>,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<NewPriceVersion>,
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
    if body.plan_id != plan_id {
        return identity::error(
            StatusCode::BAD_REQUEST,
            "plan_identifier_mismatch",
            "validation",
            false,
            correlation,
        );
    }
    let request = PublishPriceVersionRequest {
        session_token: session,
        csrf_token: csrf,
        plan_id,
        currency: body.currency,
        amount_minor: body.amount_minor,
        period_unit: body.period_unit,
        interval_count: body.interval_count,
        effective_from: body.effective_from,
        effective_until: body.effective_until.unwrap_or_default(),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match state
        .catalog_client
        .clone()
        .publish_price_version(tonic_request(request, &correlation))
        .await
    {
        Ok(response) => {
            (StatusCode::CREATED, Json(price_view(response.into_inner()))).into_response()
        }
        Err(status) => identity::grpc_error(
            status,
            "publish_price_version",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

fn price_view(p: platform_contracts::catalog_pricing::PriceVersionResponse) -> PriceView {
    PriceView {
        price_version_id: p.price_version_id,
        plan_id: p.plan_id,
        version: p.version,
        currency: p.currency,
        amount_minor: p.amount_minor,
        period_unit: p.period_unit,
        interval_count: p.interval_count,
        effective_from: p.effective_from,
        effective_until: if p.effective_until.is_empty() {
            None
        } else {
            Some(p.effective_until)
        },
        published_at: p.published_at,
    }
}
