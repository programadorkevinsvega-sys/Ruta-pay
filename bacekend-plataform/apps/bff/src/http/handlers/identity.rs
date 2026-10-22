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
    CreateApiKeyRequest, ListApiKeysRequest, RevokeApiKeyRequest,
};

const SESSION_COOKIE: &str = "payvia_tenant_session";
const CSRF_COOKIE: &str = "payvia_tenant_csrf";
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

#[utoipa::path(post, path = "/v1/identity/register", request_body = Credentials, responses((status = 201, body = RegisteredUser), (status = 409, body = ErrorEnvelope)))]
pub async fn register(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Response {
    let correlation = correlation_id(&headers);
    if !valid_origin(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "authorization",
            false,
            correlation,
        );
    }
    let started = std::time::Instant::now();
    match IdentityAccessClient::register(
        &state.identity_client,
        body.email,
        body.password,
        &correlation,
    )
    .await
    {
        Ok(response) => (
            StatusCode::CREATED,
            Json(RegisteredUser {
                user_id: response.user_id,
            }),
        )
            .into_response(),
        Err(status) => grpc_error(
            status,
            "register_tenant_user",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(post, path = "/v1/identity/session", request_body = Credentials, responses((status = 200, body = SessionStarted), (status = 401, body = ErrorEnvelope)))]
pub async fn login(
    State(state): State<std::sync::Arc<AppState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Response {
    let correlation = correlation_id(&headers);
    if !valid_origin(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "authorization",
            false,
            correlation,
        );
    }
    let started = std::time::Instant::now();
    match IdentityAccessClient::login(
        &state.identity_client,
        body.email,
        body.password,
        remote.ip().to_string(),
        &correlation,
    )
    .await
    {
        Ok(session) => {
            let mut response = (
                StatusCode::OK,
                Json(SessionStarted {
                    user_id: session.user_id,
                    expires_at: session.expires_at.clone(),
                }),
            )
                .into_response();
            if append_cookie(
                &mut response,
                format!(
                    "{SESSION_COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=43200",
                    session.session_token
                ),
            )
            .is_err()
                || append_cookie(
                    &mut response,
                    format!(
                        "{CSRF_COOKIE}={}; Path=/; Secure; SameSite=Lax; Max-Age=43200",
                        session.csrf_token
                    ),
                )
                .is_err()
            {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session_cookie_failed",
                    "internal",
                    false,
                    correlation,
                );
            }
            response
        }
        Err(status) => grpc_error(
            status,
            "create_tenant_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path = "/v1/identity/session", responses((status = 200, body = TenantSessionResponse), (status = 401, body = ErrorEnvelope)))]
pub async fn tenant_session(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some(session_token) = cookie_value(&headers, SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let Some(csrf_token) = cookie_value(&headers, CSRF_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let started = std::time::Instant::now();
    match IdentityAccessClient::validate_session(
        &state.identity_client,
        session_token,
        csrf_token,
        &correlation,
    )
    .await
    {
        Ok(session) => Json(TenantSessionResponse {
            user_id: session.user_id,
        })
        .into_response(),
        Err(status) => grpc_error(
            status,
            "validate_tenant_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(post, path = "/v1/platform/identity/session", request_body = Credentials, responses((status = 200, body = OperatorSessionResponse), (status = 401, body = ErrorEnvelope)))]
pub async fn operator_login(
    State(state): State<std::sync::Arc<AppState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Response {
    let correlation = correlation_id(&headers);
    if !valid_origin(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "authorization",
            false,
            correlation,
        );
    }
    let started = std::time::Instant::now();
    match IdentityAccessClient::operator_login(
        &state.identity_client,
        body.email,
        body.password,
        remote.ip().to_string(),
        &correlation,
    )
    .await
    {
        Ok(session) => {
            let mut response = (
                StatusCode::OK,
                Json(OperatorSessionResponse {
                    operator_id: session.user_id.clone(),
                }),
            )
                .into_response();
            if append_cookie(
                &mut response,
                format!(
                    "{OPERATOR_SESSION_COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=43200",
                    session.session_token
                ),
            )
            .is_err()
                || append_cookie(
                    &mut response,
                    format!(
                        "{OPERATOR_CSRF_COOKIE}={}; Path=/; Secure; SameSite=Lax; Max-Age=43200",
                        session.csrf_token
                    ),
                )
                .is_err()
            {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session_cookie_failed",
                    "internal",
                    false,
                    correlation,
                );
            }
            response
        }
        Err(status) => grpc_error(
            status,
            "create_operator_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path = "/v1/platform/identity/session", responses((status = 200, body = OperatorSessionResponse), (status = 401, body = ErrorEnvelope)))]
pub async fn operator_session(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some(session_token) = cookie_value(&headers, OPERATOR_SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let Some(csrf_token) = cookie_value(&headers, OPERATOR_CSRF_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    match IdentityAccessClient::validate_operator_session(
        &state.identity_client,
        session_token,
        csrf_token,
        &correlation,
    )
    .await
    {
        Ok(session) => Json(OperatorSessionResponse {
            operator_id: session.user_id,
        })
        .into_response(),
        Err(status) => grpc_error(status, "validate_operator_session", &correlation, 0),
    }
}

#[utoipa::path(delete, path = "/v1/platform/identity/session", responses((status = 204), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn operator_logout(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    if !valid_origin(&headers) || !valid_csrf(&headers, OPERATOR_CSRF_COOKIE) {
        return error(
            StatusCode::FORBIDDEN,
            "csrf_validation_failed",
            "authorization",
            false,
            correlation,
        );
    }
    let Some(session_token) = cookie_value(&headers, OPERATOR_SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let csrf_token =
        cookie_value(&headers, OPERATOR_CSRF_COOKIE).expect("operator CSRF cookie was validated");
    let started = std::time::Instant::now();
    if let Err(status) = IdentityAccessClient::validate_operator_session(
        &state.identity_client,
        session_token.clone(),
        csrf_token,
        &correlation,
    )
    .await
    {
        return grpc_error(
            status,
            "validate_operator_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        );
    }
    match IdentityAccessClient::operator_logout(&state.identity_client, session_token, &correlation)
        .await
    {
        Ok(_) => clear_session(
            response_no_content(),
            OPERATOR_SESSION_COOKIE,
            OPERATOR_CSRF_COOKIE,
        ),
        Err(status) => grpc_error(
            status,
            "revoke_operator_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(delete, path = "/v1/identity/session", responses((status = 204), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn logout(State(state): State<std::sync::Arc<AppState>>, headers: HeaderMap) -> Response {
    let correlation = correlation_id(&headers);
    let token = match validate_mutation_session(&state, &headers, &correlation).await {
        Ok(token) => token,
        Err(response) => return *response,
    };
    let started = std::time::Instant::now();
    match IdentityAccessClient::logout(&state.identity_client, token, &correlation).await {
        Ok(_) => clear_session(response_no_content(), SESSION_COOKIE, CSRF_COOKIE),
        Err(status) => grpc_error(
            status,
            "revoke_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(post, path = "/v1/tenants/{tenant_id}/api-keys", params(("tenant_id" = String, Path)), request_body = NewApiKey, responses((status = 201, body = ApiKeyCreated), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn create_api_key(
    State(state): State<std::sync::Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<NewApiKey>,
) -> Response {
    let correlation = correlation_id(&headers);
    let session_token = match validate_mutation_session(&state, &headers, &correlation).await {
        Ok(token) => token,
        Err(response) => return *response,
    };
    let started = std::time::Instant::now();
    let rpc = CreateApiKeyRequest {
        session_token,
        tenant_id: tenant_id.to_string(),
        name: body.name,
        environment: body.environment,
        scopes: body.scopes,
        correlation_id: correlation.clone(),
    };
    match IdentityAccessClient::create_api_key(&state.identity_client, rpc, &correlation).await {
        Ok(key) => (
            StatusCode::CREATED,
            Json(ApiKeyCreated {
                key_id: key.key_id,
                secret_once: key.secret_once,
                prefix: key.prefix,
                environment: key.environment,
                scopes: key.scopes,
                created_at: key.created_at,
            }),
        )
            .into_response(),
        Err(status) => grpc_error(
            status,
            "create_api_key",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path = "/v1/tenants/{tenant_id}/api-keys", params(("tenant_id" = String, Path)), responses((status = 200, body = [ApiKeyMetadataResponse]), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn list_api_keys(
    State(state): State<std::sync::Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some(session_token) = cookie_value(&headers, SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let started = std::time::Instant::now();
    let request = ListApiKeysRequest {
        session_token,
        tenant_id: tenant_id.to_string(),
        correlation_id: correlation.clone(),
    };
    match IdentityAccessClient::list_api_keys(&state.identity_client, request, &correlation).await {
        Ok(keys) => Json(
            keys.api_keys
                .into_iter()
                .map(|key| ApiKeyMetadataResponse {
                    key_id: key.key_id,
                    name: key.name,
                    prefix: key.prefix,
                    environment: key.environment,
                    scopes: key.scopes,
                    status: key.status,
                    created_at: key.created_at,
                    revoked_at: (!key.revoked_at.is_empty()).then_some(key.revoked_at),
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(status) => grpc_error(
            status,
            "list_api_keys",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(delete, path = "/v1/tenants/{tenant_id}/api-keys/{key_id}", params(("tenant_id" = String, Path), ("key_id" = String, Path)), responses((status = 204), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn revoke_api_key(
    State(state): State<std::sync::Arc<AppState>>,
    Path((tenant_id, key_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let session_token = match validate_mutation_session(&state, &headers, &correlation).await {
        Ok(token) => token,
        Err(response) => return *response,
    };
    let started = std::time::Instant::now();
    let request = RevokeApiKeyRequest {
        session_token,
        tenant_id: tenant_id.to_string(),
        key_id: key_id.to_string(),
        correlation_id: correlation.clone(),
    };
    match IdentityAccessClient::revoke_api_key(&state.identity_client, request, &correlation).await
    {
        Ok(_) => response_no_content(),
        Err(status) => grpc_error(
            status,
            "revoke_api_key",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

fn valid_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin == format!("https://{host}")
        || (host.starts_with("localhost") && origin == format!("http://{host}"))
}

fn valid_csrf(headers: &HeaderMap, csrf_cookie_name: &str) -> bool {
    let Some(cookie) = cookie_value(headers, csrf_cookie_name) else {
        return false;
    };
    let Some(header) = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    cookie.as_bytes().ct_eq(header.as_bytes()).into()
}

async fn validate_mutation_session(
    state: &AppState,
    headers: &HeaderMap,
    correlation: &str,
) -> Result<String, Box<Response>> {
    if !valid_origin(headers) || !valid_csrf(headers, CSRF_COOKIE) {
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "csrf_validation_failed",
            "authorization",
            false,
            correlation.to_owned(),
        )));
    }
    let Some(session_token) = cookie_value(headers, SESSION_COOKIE) else {
        return Err(Box::new(error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation.to_owned(),
        )));
    };
    let Some(csrf_token) = cookie_value(headers, CSRF_COOKIE) else {
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "csrf_validation_failed",
            "authorization",
            false,
            correlation.to_owned(),
        )));
    };
    let started = std::time::Instant::now();
    match IdentityAccessClient::validate_session(
        &state.identity_client,
        session_token.clone(),
        csrf_token,
        correlation,
    )
    .await
    {
        Ok(_) => Ok(session_token),
        Err(status) => Err(Box::new(grpc_error(
            status,
            "validate_tenant_session",
            correlation,
            started.elapsed().as_millis() as u64,
        ))),
    }
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name && !value.is_empty() && value.len() <= 256).then(|| value.to_owned())
        })
}

fn append_cookie(
    response: &mut Response,
    cookie: String,
) -> Result<(), axum::http::header::InvalidHeaderValue> {
    response
        .headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    Ok(())
}

fn response_no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn clear_session(mut response: Response, session_cookie: &str, csrf_cookie: &str) -> Response {
    let session_cookie_value =
        format!("{session_cookie}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0");
    let csrf_cookie_value = format!("{csrf_cookie}=; Path=/; Secure; SameSite=Lax; Max-Age=0");
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&session_cookie_value)
            .expect("known session cookie name is a valid header value"),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&csrf_cookie_value)
            .expect("known CSRF cookie name is a valid header value"),
    );
    response
}

fn grpc_error(
    status: tonic::Status,
    operation: &'static str,
    correlation: &str,
    duration_ms: u64,
) -> Response {
    let (http, code, category, retryable) = match status.code() {
        tonic::Code::InvalidArgument => (
            StatusCode::BAD_REQUEST,
            "identity_input_invalid",
            "validation",
            false,
        ),
        tonic::Code::AlreadyExists => {
            (StatusCode::CONFLICT, "identity_conflict", "conflict", false)
        }
        tonic::Code::Unauthenticated => (
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
        ),
        tonic::Code::PermissionDenied => (
            StatusCode::FORBIDDEN,
            "authorization_denied",
            "authorization",
            false,
        ),
        tonic::Code::NotFound => (
            StatusCode::NOT_FOUND,
            "resource_not_found",
            "not_found",
            false,
        ),
        tonic::Code::ResourceExhausted => (
            StatusCode::TOO_MANY_REQUESTS,
            "identity_rate_limited",
            "rate_limit",
            false,
        ),
        tonic::Code::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "identity_dependency_unavailable",
            "dependency",
            true,
        ),
        tonic::Code::DeadlineExceeded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "module_timeout",
            "timeout",
            true,
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "identity_operation_failed",
            "internal",
            false,
        ),
    };
    tracing::warn!(event.name = "bff.dependency.call.failed", component = "platform_bff", operation, dependency = "identity_access", error_code = %status.code(), error_category = category, retryable, dependency_duration_ms = duration_ms, correlation_id = %correlation, trace_id = %current_trace_id(), span_id = %current_span_id());
    error(http, code, category, retryable, correlation.to_owned())
}

fn error(
    status: StatusCode,
    code: &'static str,
    category: &'static str,
    retryable: bool,
    correlation_id: String,
) -> Response {
    let trace_id = current_trace_id();
    tracing::warn!(event.name = "bff.request.failed", component = "platform_bff", operation = "identity", error_code = code, error_category = category, retryable, correlation_id = %correlation_id, trace_id = %trace_id, span_id = %current_span_id());
    let body = ErrorEnvelope {
        error: ErrorBody {
            code,
            category,
            message: "The identity operation could not be completed.",
            correlation_id,
            trace_id,
            retryable,
        },
    };
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert("x-internal-error-code", HeaderValue::from_static(code));
    response.headers_mut().insert(
        "x-internal-error-category",
        HeaderValue::from_static(category),
    );
    response
}
