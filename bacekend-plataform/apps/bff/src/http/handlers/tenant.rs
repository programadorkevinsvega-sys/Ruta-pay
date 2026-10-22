use axum::{
    Json,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use opentelemetry::propagation::Injector;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::{
    common::grpc::tenant_registry_client::TenantRegistryClient,
    http::{handlers::identity, middleware::correlation_id::correlation_id, routes::AppState},
};
use platform_contracts::realtime::{RealtimeEvent, ReplayEventsRequest, SubscribeEventsRequest};
use platform_contracts::tenant_registry::{
    ChangeTenantStatusRequest, GetTenantRequest, ListTenantsRequest, RegisterTenantRequest,
};

const TENANT_SESSION_COOKIE: &str = "payvia_tenant_session";
const TENANT_CSRF_COOKIE: &str = "payvia_tenant_csrf";
const OPERATOR_SESSION_COOKIE: &str = "payvia_platform_session";
const OPERATOR_CSRF_COOKIE: &str = "payvia_platform_csrf";

#[derive(Deserialize)]
pub struct RealtimeQuery {
    audience: String,
    tenant_id: Option<String>,
    cursor: Option<u64>,
}

#[derive(Serialize)]
struct RealtimeEventView {
    cursor: u64,
    event_id: String,
    event_type: String,
    aggregate_id: String,
    aggregate_version: u64,
    tenant_id: String,
    correlation_id: String,
    payload_json: String,
}

#[derive(Serialize)]
struct RealtimeReplayView {
    events: Vec<RealtimeEventView>,
    next_cursor: u64,
    has_more: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct NewTenant {
    display_name: String,
    slug: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ChangeTenantStatus {
    status: String,
    reason: String,
    expected_version: i64,
}

#[derive(Deserialize)]
pub struct TenantQuery {
    query: Option<String>,
    limit: Option<i32>,
    offset: Option<i32>,
}

#[derive(Serialize, ToSchema)]
pub struct TenantView {
    tenant_id: String,
    display_name: String,
    slug: String,
    status: String,
    status_version: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct TenantList {
    tenants: Vec<TenantView>,
    total: i64,
}

#[utoipa::path(post, path = "/v1/tenants", request_body = NewTenant, responses((status = 201, body = TenantView), (status = 409, body = crate::http::dto::error_envelope::ErrorEnvelope)))]
pub async fn register(
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

#[utoipa::path(get, path = "/v1/tenants/{tenant_id}", params(("tenant_id" = String, Path)), responses((status = 200, body = TenantView)))]
pub async fn get(
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

#[utoipa::path(get, path = "/v1/platform/tenants", params(("query" = Option<String>, Query), ("limit" = Option<i32>, Query), ("offset" = Option<i32>, Query)), responses((status = 200, body = TenantList)))]
pub async fn list_for_operator(
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

#[utoipa::path(post, path = "/v1/platform/tenants/{tenant_id}/status", request_body = ChangeTenantStatus, responses((status = 200, body = TenantView)))]
pub async fn change_status(
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

pub async fn replay_events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RealtimeQuery>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some((session, csrf)) = realtime_credentials(&headers, &query.audience) else {
        return auth_error(correlation);
    };
    let tenant_id = query.tenant_id.unwrap_or_default();
    let request = tonic_request(
        ReplayEventsRequest {
            session_token: session,
            csrf_token: csrf,
            audience: query.audience,
            tenant_id,
            after_cursor: query.cursor.unwrap_or(0),
            limit: 250,
            correlation_id: correlation.clone(),
        },
        &correlation,
    );
    let started = std::time::Instant::now();
    let mut client = state.realtime_client.clone();
    match client.replay_events(request).await {
        Ok(response) => {
            let response = response.into_inner();
            Json(RealtimeReplayView {
                events: response
                    .events
                    .into_iter()
                    .map(realtime_event_view)
                    .collect(),
                next_cursor: response.next_cursor,
                has_more: response.has_more,
            })
            .into_response()
        }
        Err(status) => tenant_grpc_error(
            status,
            "replay_realtime_events",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

pub async fn websocket_events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RealtimeQuery>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let correlation = correlation_id(&headers);
    if !identity::valid_origin(&headers) {
        return csrf_error(correlation);
    }
    let Some((session, csrf)) = realtime_credentials(&headers, &query.audience) else {
        return auth_error(correlation);
    };
    let audience = query.audience.clone();
    let tenant_id = query.tenant_id.unwrap_or_default();
    let request = tonic_request(
        SubscribeEventsRequest {
            session_token: session,
            csrf_token: csrf,
            audience: query.audience,
            tenant_id,
            after_cursor: query.cursor.unwrap_or(0),
            correlation_id: correlation.clone(),
        },
        &correlation,
    );
    let mut client = state.realtime_client.clone();
    match client.subscribe_events(request).await {
        Ok(stream) => {
            tracing::info!(event.name = "bff.realtime.websocket.accepted", component = "platform_bff", operation = "subscribe_realtime_events", result = "success", audience = %audience, correlation_id = %correlation, trace_id = %payvia_observability::current_trace_id(), span_id = %payvia_observability::current_span_id());
            websocket.on_upgrade(move |socket| {
                bridge_realtime_stream(socket, stream.into_inner(), correlation)
            })
        }
        Err(status) => tenant_grpc_error(status, "subscribe_realtime_events", &correlation, 0),
    }
}

async fn bridge_realtime_stream(
    mut socket: WebSocket,
    mut stream: tonic::Streaming<RealtimeEvent>,
    correlation_id: String,
) {
    loop {
        tokio::select! {
            event = stream.message() => match event {
                Ok(Some(event)) => {
                    let payload = realtime_event_view(event);
                    let message = match serde_json::to_string(&payload) {
                        Ok(message) => message,
                        Err(_) => {
                            tracing::error!(event.name = "bff.realtime.serialize.failed", component = "platform_bff", operation = "serialize_realtime_event", error_code = "realtime_event_serialization_failed", error_category = "internal", correlation_id = %correlation_id);
                            break;
                        }
                    };
                    if socket.send(Message::Text(message.into())).await.is_err() { break; }
                }
                Ok(None) => break,
                Err(status) => {
                    tracing::warn!(event.name = "bff.realtime.stream.failed", component = "platform_bff", operation = "read_realtime_stream", dependency = "realtime", error_code = %status.code(), error_category = "dependency", correlation_id = %correlation_id, trace_id = %payvia_observability::current_trace_id(), span_id = %payvia_observability::current_span_id());
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(Message::Ping(bytes))) => if socket.send(Message::Pong(bytes)).await.is_err() { break; },
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) => {},
            }
        }
    }
}

fn realtime_credentials(headers: &HeaderMap, audience: &str) -> Option<(String, String)> {
    let (session_cookie, csrf_cookie) = match audience {
        "tenant" => (TENANT_SESSION_COOKIE, TENANT_CSRF_COOKIE),
        "platform" => (OPERATOR_SESSION_COOKIE, OPERATOR_CSRF_COOKIE),
        _ => return None,
    };
    Some((
        identity::cookie_value(headers, session_cookie)?,
        identity::cookie_value(headers, csrf_cookie)?,
    ))
}

fn tonic_request<T>(body: T, correlation: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(body);
    if let Ok(value) = tonic::metadata::MetadataValue::try_from(correlation) {
        request.metadata_mut().insert("x-correlation-id", value);
    }
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &opentelemetry::Context::current(),
            &mut GrpcMetadataInjector(request.metadata_mut()),
        );
    });
    request.set_timeout(std::time::Duration::from_secs(3));
    request
}

struct GrpcMetadataInjector<'a>(&'a mut tonic::metadata::MetadataMap);

impl Injector for GrpcMetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(key), Ok(value)) = (
            tonic::metadata::MetadataKey::from_bytes(key.as_bytes()),
            tonic::metadata::MetadataValue::try_from(value),
        ) {
            self.0.insert(key, value);
        }
    }
}

fn realtime_event_view(event: RealtimeEvent) -> RealtimeEventView {
    RealtimeEventView {
        cursor: event.cursor,
        event_id: event.event_id,
        event_type: event.event_type,
        aggregate_id: event.aggregate_id,
        aggregate_version: event.aggregate_version,
        tenant_id: event.tenant_id,
        correlation_id: event.correlation_id,
        payload_json: event.payload_json,
    }
}

fn csrf_value(headers: &HeaderMap, cookie_name: &str) -> Option<(String, String)> {
    if !identity::valid_origin(headers) || !identity::valid_csrf(headers, cookie_name) {
        return None;
    }
    let session_cookie = if cookie_name == TENANT_CSRF_COOKIE {
        TENANT_SESSION_COOKIE
    } else {
        OPERATOR_SESSION_COOKIE
    };
    Some((
        identity::cookie_value(headers, session_cookie)?,
        identity::cookie_value(headers, cookie_name)?,
    ))
}

fn map_tenant(response: platform_contracts::tenant_registry::TenantResponse) -> TenantView {
    TenantView {
        tenant_id: response.tenant_id,
        display_name: response.display_name,
        slug: response.slug,
        status: response.status,
        status_version: response.status_version,
        created_at: response.created_at,
        updated_at: response.updated_at,
    }
}

fn tenant_grpc_error(
    status: tonic::Status,
    operation: &'static str,
    correlation: &str,
    duration_ms: u64,
) -> Response {
    let (http_status, code, category, retryable) = match status.code() {
        tonic::Code::InvalidArgument => (
            StatusCode::BAD_REQUEST,
            "tenant_input_invalid",
            "validation",
            false,
        ),
        tonic::Code::AlreadyExists => (StatusCode::CONFLICT, "tenant_conflict", "conflict", false),
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
            "tenant_not_found",
            "not_found",
            false,
        ),
        tonic::Code::Aborted | tonic::Code::FailedPrecondition => (
            StatusCode::CONFLICT,
            "tenant_state_conflict",
            "conflict",
            false,
        ),
        tonic::Code::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant_dependency_unavailable",
            "dependency",
            true,
        ),
        tonic::Code::DeadlineExceeded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant_module_timeout",
            "timeout",
            true,
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "tenant_operation_failed",
            "internal",
            false,
        ),
    };
    tracing::warn!(event.name = "bff.dependency.call.failed", component = "platform_bff", operation, dependency = "tenant_registry", grpc_code = %status.code(), error_code = code, error_category = category, retryable, dependency_duration_ms = duration_ms, correlation_id = %correlation, trace_id = %payvia_observability::current_trace_id(), span_id = %payvia_observability::current_span_id());
    identity::error(
        http_status,
        code,
        category,
        retryable,
        correlation.to_owned(),
    )
}

fn auth_error(correlation: String) -> Response {
    identity::error(
        StatusCode::UNAUTHORIZED,
        "authentication_failed",
        "authentication",
        false,
        correlation,
    )
}

fn csrf_error(correlation: String) -> Response {
    identity::error(
        StatusCode::FORBIDDEN,
        "csrf_validation_failed",
        "authorization",
        false,
        correlation,
    )
}
