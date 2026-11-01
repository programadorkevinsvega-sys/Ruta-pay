use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::http::{handlers::identity, middleware::correlation_id::correlation_id, routes::AppState};
use platform_contracts::entitlements_quotas::{CapabilityInput, SetPlanCapabilitiesRequest};
use super::{shared::{operator_credentials, tonic_request}, types::SetCapabilities};

#[utoipa::path(post, path="/v1/platform/entitlements/capabilities", request_body=SetCapabilities, responses((status=200)))]
pub async fn set_capabilities(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SetCapabilities>,
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
    if body.capabilities.len() > 100 {
        return identity::error(
            StatusCode::BAD_REQUEST,
            "capability_count_invalid",
            "validation",
            false,
            correlation,
        );
    }
    let capabilities = body
        .capabilities
        .into_iter()
        .map(|c| CapabilityInput {
            capability_key: c.capability_key,
            enabled: c.enabled,
            limit_value: c.limit_value,
            unit: c.unit,
            overage_policy: c.overage_policy,
            environment: c.environment,
            quota_period: c.quota_period,
        })
        .collect();
    let request = SetPlanCapabilitiesRequest {
        session_token: session,
        csrf_token: csrf,
        price_version_id: body.price_version_id,
        capabilities,
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match state
        .entitlements_client
        .clone()
        .set_plan_capabilities(tonic_request(request, &correlation))
        .await
    {
        Ok(response) => {
            Json(serde_json::json!({"capabilities_saved":response.into_inner().capabilities_saved}))
                .into_response()
        }
        Err(status) => identity::grpc_error(
            status,
            "set_plan_capabilities",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}
