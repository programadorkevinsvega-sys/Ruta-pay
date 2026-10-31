use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::http::{
    handlers::identity, middleware::correlation_id::correlation_id, routes::AppState,
};
use platform_contracts::{
    catalog_pricing::{CreatePlanRequest, ListPlansRequest, PublishPriceVersionRequest},
    entitlements_quotas::{
        AssignTenantPlanRequest, CapabilityInput, GetTenantEntitlementRequest,
        ListTenantEntitlementsRequest, SetPlanCapabilitiesRequest,
    },
};

const TENANT_SESSION: &str = "payvia_tenant_session";
const TENANT_CSRF: &str = "payvia_tenant_csrf";
const OPERATOR_SESSION: &str = "payvia_platform_session";
const OPERATOR_CSRF: &str = "payvia_platform_csrf";

#[derive(Deserialize, ToSchema)]
pub struct NewPlan {
    slug: String,
    name: String,
    description: String,
}

#[derive(Deserialize, ToSchema)]
pub struct NewPriceVersion {
    plan_id: String,
    currency: String,
    amount_minor: i64,
    period_unit: String,
    interval_count: u32,
    effective_from: String,
    effective_until: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CapabilityDefinition {
    capability_key: String,
    enabled: bool,
    limit_value: Option<i64>,
    unit: String,
    overage_policy: String,
    environment: String,
    quota_period: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SetCapabilities {
    price_version_id: String,
    capabilities: Vec<CapabilityDefinition>,
}

#[derive(Deserialize, ToSchema)]
pub struct AssignPlan {
    tenant_id: String,
    price_version_id: String,
    effective_from: String,
    effective_until: Option<String>,
}

#[derive(Deserialize)]
pub struct EntitlementQuery {
    environment: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PlanView {
    plan_id: String,
    slug: String,
    name: String,
    description: String,
    status: String,
    created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PriceView {
    price_version_id: String,
    plan_id: String,
    version: u32,
    currency: String,
    amount_minor: i64,
    period_unit: String,
    interval_count: u32,
    effective_from: String,
    effective_until: Option<String>,
    published_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct EntitlementView {
    tenant_id: String,
    price_version_id: String,
    capability_key: String,
    enabled: bool,
    limit_value: Option<i64>,
    consumed: i64,
    remaining: Option<i64>,
    allowed: bool,
    reason: String,
    environment: String,
}

#[utoipa::path(post, path="/v1/platform/catalog/plans", request_body=NewPlan, responses((status=201, body=PlanView)))]
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

#[utoipa::path(post, path="/v1/platform/entitlements/assignments", request_body=AssignPlan, responses((status=201)))]
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
pub async fn list_entitlements(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
    Query(query): Query<EntitlementQuery>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    if headers.contains_key("origin") && !identity::valid_origin(&headers) {
        return identity::error(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "authorization",
            false,
            correlation,
        );
    }
    let (Some(session), Some(csrf)) = (
        identity::cookie_value(&headers, TENANT_SESSION),
        identity::cookie_value(&headers, TENANT_CSRF),
    ) else {
        return identity::error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let request = ListTenantEntitlementsRequest {
        session_token: session,
        csrf_token: csrf,
        tenant_id,
        environment: query.environment.unwrap_or_else(|| "test".to_owned()),
        effective_at: Utc::now().to_rfc3339(),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match state
        .entitlements_client
        .clone()
        .list_tenant_entitlements(tonic_request(request, &correlation))
        .await
    {
        Ok(response) => Json(
            response
                .into_inner()
                .entitlements
                .into_iter()
                .map(entitlement_view)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(status) => identity::grpc_error(
            status,
            "list_tenant_entitlements",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path="/v1/tenants/{tenant_id}/entitlements/{capability_key}", params(("tenant_id"=String,Path),("capability_key"=String,Path),("environment"=Option<String>,Query)), responses((status=200, body=EntitlementView)))]
pub async fn get_entitlement(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, capability_key)): Path<(String, String)>,
    Query(query): Query<EntitlementQuery>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    if headers.contains_key("origin") && !identity::valid_origin(&headers) {
        return identity::error(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "authorization",
            false,
            correlation,
        );
    }
    let (Some(session), Some(csrf)) = (
        identity::cookie_value(&headers, TENANT_SESSION),
        identity::cookie_value(&headers, TENANT_CSRF),
    ) else {
        return identity::error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let request = GetTenantEntitlementRequest {
        session_token: session,
        csrf_token: csrf,
        tenant_id,
        capability_key,
        environment: query.environment.unwrap_or_else(|| "test".to_owned()),
        effective_at: Utc::now().to_rfc3339(),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match state
        .entitlements_client
        .clone()
        .get_tenant_entitlement(tonic_request(request, &correlation))
        .await
    {
        Ok(response) => Json(entitlement_view(response.into_inner())).into_response(),
        Err(status) => identity::grpc_error(
            status,
            "get_tenant_entitlement",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

fn operator_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    if !identity::valid_origin(headers) || !identity::valid_csrf(headers, OPERATOR_CSRF) {
        return None;
    }
    Some((
        identity::cookie_value(headers, OPERATOR_SESSION)?,
        identity::cookie_value(headers, OPERATOR_CSRF)?,
    ))
}
fn tonic_request<T>(body: T, correlation: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(body);
    if let Ok(value) = tonic::metadata::MetadataValue::try_from(correlation) {
        request.metadata_mut().insert("x-correlation-id", value);
    }
    request.set_timeout(std::time::Duration::from_secs(3));
    request
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
fn entitlement_view(
    e: platform_contracts::entitlements_quotas::TenantEntitlementResponse,
) -> EntitlementView {
    EntitlementView {
        tenant_id: e.tenant_id,
        price_version_id: e.price_version_id,
        capability_key: e.capability_key,
        enabled: e.enabled,
        limit_value: e.limit_value,
        consumed: e.consumed,
        remaining: e.remaining,
        allowed: e.allowed,
        reason: e.reason,
        environment: e.environment,
    }
}
