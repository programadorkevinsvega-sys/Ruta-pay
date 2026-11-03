use axum::{
    Json,
    extract::{Query, State, ws::{Message, WebSocket, WebSocketUpgrade}},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use opentelemetry::propagation::Injector;
use std::sync::Arc;

use crate::{
    http::{handlers::identity, middleware::correlation_id::correlation_id, routes::AppState},
};
use platform_contracts::realtime::{RealtimeEvent, ReplayEventsRequest, SubscribeEventsRequest};

use super::{
    models::{RealtimeEventView, RealtimeQuery, RealtimeReplayView},
    support::{auth_error, csrf_error, tenant_grpc_error,
        OPERATOR_CSRF_COOKIE, OPERATOR_SESSION_COOKIE, TENANT_CSRF_COOKIE,
        TENANT_SESSION_COOKIE},
};

pub async fn replay_events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RealtimeQuery>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    if !matches!(query.audience.as_str(), "tenant" | "platform") {
        return tenant_grpc_error(
            tonic::Status::invalid_argument("realtime audience is invalid"),
            "replay_realtime_events", &correlation, 0,
        );
    }
    let Some((session, csrf)) = realtime_credentials(&headers, &query.audience) else {
        return auth_error(correlation);
    };
    let request = tonic_request(ReplayEventsRequest {
        session_token: session,
        csrf_token: csrf,
        audience: query.audience,
        tenant_id: query.tenant_id.unwrap_or_default(),
        after_cursor: query.cursor.unwrap_or(0),
        limit: 250,
        correlation_id: correlation.clone(),
    }, &correlation);
    let started = std::time::Instant::now();
    let mut client = state.realtime_client.clone();
    match client.replay_events(request).await {
        Ok(response) => {
            let response = response.into_inner();
            Json(RealtimeReplayView {
                events: response.events.into_iter().map(realtime_event_view).collect(),
                next_cursor: response.next_cursor,
                has_more: response.has_more,
            }).into_response()
        }
        Err(status) => tenant_grpc_error(
            status, "replay_realtime_events", &correlation,
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
    if !matches!(query.audience.as_str(), "tenant" | "platform") {
        return tenant_grpc_error(
            tonic::Status::invalid_argument("realtime audience is invalid"),
            "subscribe_realtime_events", &correlation, 0,
        );
    }
    let Some((session, csrf)) = realtime_credentials(&headers, &query.audience) else {
        return auth_error(correlation);
    };
    let audience = query.audience.clone();
    let request = tonic_request(SubscribeEventsRequest {
        session_token: session,
        csrf_token: csrf,
        audience: query.audience,
        tenant_id: query.tenant_id.unwrap_or_default(),
        after_cursor: query.cursor.unwrap_or(0),
        correlation_id: correlation.clone(),
    }, &correlation);
    let mut client = state.realtime_client.clone();
    match client.subscribe_events(request).await {
        Ok(stream) => {
            tracing::info!(
                event.name = "bff.realtime.websocket.accepted",
                component = "platform_bff", operation = "subscribe_realtime_events",
                result = "success", audience = %audience, correlation_id = %correlation,
                trace_id = %payvia_observability::current_trace_id(),
                span_id = %payvia_observability::current_span_id()
            );
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
                            tracing::error!(
                                event.name = "bff.realtime.serialize.failed",
                                component = "platform_bff",
                                operation = "serialize_realtime_event",
                                error_code = "realtime_event_serialization_failed",
                                error_category = "internal",
                                correlation_id = %correlation_id
                            );
                            break;
                        }
                    };
                    if socket.send(Message::Text(message.into())).await.is_err() { break; }
                }
                Ok(None) => break,
                Err(status) => {
                    tracing::warn!(
                        event.name = "bff.realtime.stream.failed",
                        component = "platform_bff", operation = "read_realtime_stream",
                        dependency = "realtime", error_code = %status.code(),
                        error_category = "dependency", correlation_id = %correlation_id,
                        trace_id = %payvia_observability::current_trace_id(),
                        span_id = %payvia_observability::current_span_id()
                    );
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
