use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use std::sync::Arc;

use super::{
    shared::tonic_request,
    types::{EntitlementQuery, EntitlementView},
};
use crate::http::{
    handlers::identity, middleware::correlation_id::correlation_id, routes::AppState,
};
use platform_contracts::entitlements_quotas::{
    GetTenantEntitlementRequest, ListTenantEntitlementsRequest,
};

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
        identity::cookie_value(&headers, identity::SESSION_COOKIE),
        identity::cookie_value(&headers, identity::CSRF_COOKIE),
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
        identity::cookie_value(&headers, identity::SESSION_COOKIE),
        identity::cookie_value(&headers, identity::CSRF_COOKIE),
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
