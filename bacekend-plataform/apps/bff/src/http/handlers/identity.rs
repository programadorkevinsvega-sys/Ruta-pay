use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use subtle::ConstantTimeEq;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    common::grpc::identity_access_client::IdentityAccessClient,
    http::{
        dto::error_envelope::{ErrorBody, ErrorEnvelope},
        middleware::correlation_id::correlation_id,
        routes::AppState,
    },
};
use payvia_observability::{current_span_id, current_trace_id};
use platform_contracts::identity_access::{
    CreateApiKeyRequest, ListApiKeysRequest, ListTenantMembershipsRequest, RevokeApiKeyRequest,
};

pub(crate) const SESSION_COOKIE: &str = "payvia_tenant_session";
pub(crate) const CSRF_COOKIE: &str = "payvia_tenant_csrf";
const OPERATOR_SESSION_COOKIE: &str = "payvia_platform_session";
const OPERATOR_CSRF_COOKIE: &str = "payvia_platform_csrf";

#[derive(Deserialize, ToSchema)]
pub struct Credentials {
    email: String,
    password: String,
}

#[derive(Serialize, ToSchema)]
pub struct RegisteredUser {
    user_id: String,
}

#[derive(Serialize, ToSchema)]
pub struct SessionStarted {
    user_id: String,
    expires_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct TenantSessionResponse {
    user_id: String,
}

#[derive(Serialize, ToSchema)]
pub struct OperatorSessionResponse {
    operator_id: String,
}

#[derive(Serialize, ToSchema)]
pub struct TenantMembershipResponse {
    tenant_id: String,
    role: String,
    status: String,
}

#[derive(Serialize, ToSchema)]
pub struct TenantMembershipListResponse {
    memberships: Vec<TenantMembershipResponse>,
}

#[derive(Deserialize, ToSchema)]
pub struct NewApiKey {
    name: String,
    environment: String,
    scopes: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiKeyCreated {
    key_id: String,
    secret_once: String,
    prefix: String,
    environment: String,
    scopes: Vec<String>,
    created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct ApiKeyMetadataResponse {
    key_id: String,
    name: String,
    prefix: String,
    environment: String,
    scopes: Vec<String>,
    status: String,
    created_at: String,
    revoked_at: Option<String>,
}

pub(super) mod api_keys;
mod errors;
mod memberships;
pub(super) mod operator;
pub(super) mod registration;
mod security;
pub(super) mod sessions;

pub use api_keys::{create_api_key, list_api_keys, revoke_api_key};
pub(crate) use errors::{error, grpc_error};
pub use memberships::tenant_memberships;
pub use operator::{logout, operator_login, operator_logout, operator_session};
pub use registration::register;
use security::{
    append_cookie, clear_session, response_no_content,
    validate_mutation_session,
};
pub(crate) use security::{cookie_value, valid_csrf, valid_origin};
pub use sessions::{login, tenant_session};

#[cfg(test)]
mod tests;
