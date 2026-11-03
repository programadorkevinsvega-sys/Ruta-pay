mod crud;
mod models;
mod realtime;
mod support;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use std::sync::Arc;

use crate::http::routes::AppState;

pub use models::{ChangeTenantStatus, NewTenant, TenantList, TenantQuery, TenantView};

#[utoipa::path(post, path = "/v1/tenants", request_body = NewTenant, responses((status = 201, body = TenantView), (status = 409, body = crate::http::dto::error_envelope::ErrorEnvelope)))]
pub async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NewTenant>,
) -> Response {
    crud::register(State(state), headers, Json(body)).await
}

#[utoipa::path(get, path = "/v1/tenants/{tenant_id}", params(("tenant_id" = String, Path)), responses((status = 200, body = TenantView)))]
pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    crud::get(State(state), Path(tenant_id), headers).await
}

#[utoipa::path(get, path = "/v1/platform/tenants", params(("query" = Option<String>, Query), ("limit" = Option<i32>, Query), ("offset" = Option<i32>, Query)), responses((status = 200, body = TenantList)))]
pub async fn list_for_operator(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TenantQuery>,
    headers: HeaderMap,
) -> Response {
    crud::list_for_operator(State(state), Query(query), headers).await
}

#[utoipa::path(post, path = "/v1/platform/tenants/{tenant_id}/status", request_body = ChangeTenantStatus, responses((status = 200, body = TenantView)))]
pub async fn change_status(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ChangeTenantStatus>,
) -> Response {
    crud::change_status(State(state), Path(tenant_id), headers, Json(body)).await
}

pub use realtime::{replay_events, websocket_events};
