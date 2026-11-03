use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::{
    common::grpc::tenant_registry_client::TenantRegistryClient,
    http::{handlers::identity, middleware::correlation_id::correlation_id, routes::AppState},
};
use platform_contracts::tenant_registry::{
    ChangeTenantStatusRequest, GetTenantRequest, ListTenantsRequest, RegisterTenantRequest,
};

use super::{
    models::{ChangeTenantStatus, NewTenant, TenantList, TenantQuery},
    support::{
        OPERATOR_CSRF_COOKIE, OPERATOR_SESSION_COOKIE, TENANT_CSRF_COOKIE, TENANT_SESSION_COOKIE,
        auth_error, csrf_error, csrf_value, map_tenant, tenant_grpc_error,
    },
};

pub(super) async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NewTenant>,
) -> Response {
    let correlation = correlation_id(&headers);
    let csrf = csrf_value(&headers, TENANT_CSRF_COOKIE);
    let Some((session, csrf)) = csrf else {
        return csrf_error(correlation);
    };
    let request = RegisterTenantRequest {
        session_token: session,
        display_name: body.display_name,
        slug: body.slug,
        correlation_id: correlation.clone(),
        csrf_token: csrf,
        legal_identification_type: body.legal_identification_type,
        legal_identification_number: body.legal_identification_number,
    };
    let started = std::time::Instant::now();
    match TenantRegistryClient::register(&state.tenant_client, request, &correlation).await {
        Ok(response) => (StatusCode::CREATED, Json(map_tenant(response))).into_response(),
        Err(status) => tenant_grpc_error(
            status,
            "register_tenant",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    if let (Some(session), Some(csrf)) = (
        identity::cookie_value(&headers, TENANT_SESSION_COOKIE),
        identity::cookie_value(&headers, TENANT_CSRF_COOKIE),
    ) {
        return get_with_session(&state, tenant_id, session, csrf, correlation).await;
    }
    if let (Some(session), Some(csrf)) = (
        identity::cookie_value(&headers, OPERATOR_SESSION_COOKIE),
        identity::cookie_value(&headers, OPERATOR_CSRF_COOKIE),
    ) {
        return get_with_session(&state, tenant_id, session, csrf, correlation).await;
    }
    auth_error(correlation)
}

async fn get_with_session(
    state: &AppState,
    tenant_id: String,
    session: String,
    csrf: String,
    correlation: String,
) -> Response {
    let request = GetTenantRequest {
        session_token: session,
        tenant_id,
        correlation_id: correlation.clone(),
        csrf_token: csrf,
    };
    let started = std::time::Instant::now();
    match TenantRegistryClient::get(&state.tenant_client, request, &correlation).await {
        Ok(response) => Json(map_tenant(response)).into_response(),
        Err(status) => tenant_grpc_error(
            status,
            "get_tenant",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

pub(super) async fn list_for_operator(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TenantQuery>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    if !identity::valid_origin(&headers) && headers.contains_key("origin") {
        return csrf_error(correlation);
    }
    let Some(session) = identity::cookie_value(&headers, OPERATOR_SESSION_COOKIE) else {
        return auth_error(correlation);
    };
    let Some(csrf) = identity::cookie_value(&headers, OPERATOR_CSRF_COOKIE) else {
        return csrf_error(correlation);
    };
    let request = ListTenantsRequest {
        session_token: session,
        csrf_token: csrf,
        query: query.query.unwrap_or_default(),
        limit: query.limit.unwrap_or(50),
        offset: query.offset.unwrap_or(0),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match TenantRegistryClient::list(&state.tenant_client, request, &correlation).await {
        Ok(result) => Json(TenantList {
            tenants: result.tenants.into_iter().map(map_tenant).collect(),
            total: result.total,
        })
        .into_response(),
        Err(status) => tenant_grpc_error(
            status,
            "list_tenants",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

pub(super) async fn change_status(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ChangeTenantStatus>,
) -> Response {
    let correlation = correlation_id(&headers);
    if !identity::valid_origin(&headers) || !identity::valid_csrf(&headers, OPERATOR_CSRF_COOKIE) {
        return csrf_error(correlation);
    }
    let Some(session) = identity::cookie_value(&headers, OPERATOR_SESSION_COOKIE) else {
        return auth_error(correlation);
    };
    let Some(csrf) = identity::cookie_value(&headers, OPERATOR_CSRF_COOKIE) else {
        return csrf_error(correlation);
    };
    let request = ChangeTenantStatusRequest {
        session_token: session,
        tenant_id,
        status: body.status,
        reason: body.reason,
        correlation_id: correlation.clone(),
        csrf_token: csrf,
        expected_version: body.expected_version,
    };
    let started = std::time::Instant::now();
    match TenantRegistryClient::change_status(&state.tenant_client, request, &correlation).await {
        Ok(response) => Json(map_tenant(response)).into_response(),
        Err(status) => tenant_grpc_error(
            status,
            "change_tenant_status",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}
