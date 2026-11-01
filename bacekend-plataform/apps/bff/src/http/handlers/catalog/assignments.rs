use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::http::{handlers::identity, middleware::correlation_id::correlation_id, routes::AppState};
use platform_contracts::entitlements_quotas::AssignTenantPlanRequest;
use super::{shared::{operator_credentials, tonic_request}, types::AssignPlan};

pub async fn assign_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AssignPlan>,
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
    let request = AssignTenantPlanRequest {
        session_token: session,
        csrf_token: csrf,
        tenant_id: body.tenant_id,
        price_version_id: body.price_version_id,
        effective_from: body.effective_from,
        effective_until: body.effective_until.unwrap_or_default(),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match state
        .entitlements_client
        .clone()
        .assign_tenant_plan(tonic_request(request, &correlation))
        .await
    {
        Ok(response) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"assignment_id":response.into_inner().assignment_id})),
        )
            .into_response(),
        Err(status) => identity::grpc_error(
            status,
            "assign_tenant_plan",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path="/v1/tenants/{tenant_id}/entitlements", params(("tenant_id"=String,Path),("environment"=Option<String>,Query)), responses((status=200, body=Vec<EntitlementView>)))]
