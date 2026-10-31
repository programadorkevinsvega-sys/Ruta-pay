use std::{pin::Pin, time::Duration};

use opentelemetry::{global, trace::TraceContextExt};
use platform_contracts::{
    identity_access::{
        AuthorizeTenantOperationRequest, ValidateOperatorSessionRequest,
        identity_access_client::IdentityAccessClient,
    },
    realtime::{
        RealtimeEvent, ReplayEventsRequest, ReplayEventsResponse, SubscribeEventsRequest,
        realtime_gateway_server::RealtimeGateway,
    },
};
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::infrastructure::persistence::{self, JournalError};

const SIGNAL_CHANNEL: &str = "payvia:platform:realtime:v1";
const MAX_REPLAY_LIMIT: u32 = 250;
const BATCH_SIZE: i64 = 100;

#[derive(Clone)]
pub struct RealtimeGatewayApi {
    pool: PgPool,
    redis_client: redis::Client,
    identity_client: IdentityAccessClient<tonic::transport::Channel>,
}

impl RealtimeGatewayApi {
    pub fn new(
        pool: PgPool,
        redis_client: redis::Client,
        identity_client: IdentityAccessClient<tonic::transport::Channel>,
    ) -> Self {
        Self {
            pool,
            redis_client,
            identity_client,
        }
    }

    async fn authorize(
        &self,
        token: &str,
        csrf: &str,
        audience: &str,
        tenant_id: Option<Uuid>,
        correlation_id: &str,
    ) -> Result<(), Status> {
        let mut identity = self.identity_client.clone();
        match audience {
            "platform" => identity
                .validate_operator_session(ValidateOperatorSessionRequest {
                    session_token: token.to_owned(),
                    csrf_token: csrf.to_owned(),
                    correlation_id: correlation_id.to_owned(),
                })
                .await
                .map(|_| ()),
            "tenant" => {
                let tenant_id = tenant_id
                    .ok_or_else(|| Status::invalid_argument("tenant scope is required"))?;
                identity
                    .authorize_tenant_operation(AuthorizeTenantOperationRequest {
                        session_token: token.to_owned(),
                        csrf_token: csrf.to_owned(),
                        tenant_id: tenant_id.to_string(),
                        permission: "tenant.read".to_owned(),
                        correlation_id: correlation_id.to_owned(),
                    })
                    .await
                    .map(|_| ())
            }
            _ => Err(Status::invalid_argument("realtime audience is invalid")),
        }
    }
}

#[tonic::async_trait]
impl RealtimeGateway for RealtimeGatewayApi {
    type SubscribeEventsStream = Pin<Box<dyn Stream<Item = Result<RealtimeEvent, Status>> + Send>>;

    async fn replay_events(
        &self,
        request: Request<ReplayEventsRequest>,
    ) -> Result<Response<ReplayEventsResponse>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = request_span(&request, "replay_events", &correlation_id);
        let body = request.into_inner();
        async move {
            let tenant_id = parse_scope(&body.audience, &body.tenant_id)?;
            self.authorize(
                &body.session_token,
                &body.csrf_token,
                &body.audience,
                tenant_id,
                &correlation_id,
            )
            .await?;
            if body.limit == 0
                || body.limit > MAX_REPLAY_LIMIT
                || body.after_cursor > i64::MAX as u64
            {
                return Err(Status::invalid_argument(
                    "realtime cursor request is invalid",
                ));
            }
            let events = persistence::replay(
                &self.pool,
                &body.audience,
                tenant_id,
                body.after_cursor as i64,
                i64::from(body.limit),
            )
            .await
            .map_err(map_journal_error)?;
            let next_cursor = events
                .last()
                .map_or(body.after_cursor, |event| event.cursor as u64);
            let has_more = events.len() == body.limit as usize;
            Ok(Response::new(ReplayEventsResponse {
                events: events.into_iter().map(to_proto).collect(),
                next_cursor,
                has_more,
            }))
        }
        .instrument(span)
        .await
    }

    async fn subscribe_events(
        &self,
        request: Request<SubscribeEventsRequest>,
    ) -> Result<Response<Self::SubscribeEventsStream>, Status> {
        let correlation_id = request.get_ref().correlation_id.clone();
        let span = request_span(&request, "subscribe_events", &correlation_id);
        let body = request.into_inner();
        let tenant_id = parse_scope(&body.audience, &body.tenant_id)?;
        self.authorize(
            &body.session_token,
            &body.csrf_token,
            &body.audience,
            tenant_id,
            &correlation_id,
        )
        .await?;
        if body.after_cursor > i64::MAX as u64 {
            return Err(Status::invalid_argument(
                "realtime cursor request is invalid",
            ));
        }
        // Validate the requested cursor before accepting the WebSocket upgrade.
        // A future cursor or a retained-journal gap must be explicit to the client.
        persistence::replay(
            &self.pool,
            &body.audience,
            tenant_id,
            body.after_cursor as i64,
            BATCH_SIZE,
        )
        .await
        .map_err(map_journal_error)?;
        let (sender, receiver) = mpsc::channel(128);
        let api = self.clone();
        tokio::spawn(async move {
            let stream_span = tracing::info_span!("realtime.subscription", correlation_id = %correlation_id, audience = %body.audience);
            async move {
                let mut cursor = body.after_cursor as i64;
                loop {
                    let mut pubsub = match api.redis_client.get_async_pubsub().await {
                        Ok(pubsub) => pubsub,
                        Err(error) => {
                            tracing::warn!(event.name = "realtime.redis.subscribe.failed", component = "realtime", operation = "connect_pubsub", dependency = "redis", error_code = "redis_pubsub_unavailable", error_category = "connectivity", retryable = true, error_class = %error, correlation_id = %body.correlation_id);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    };
                    if let Err(error) = pubsub.subscribe(SIGNAL_CHANNEL).await {
                        tracing::warn!(event.name = "realtime.redis.subscribe.failed", component = "realtime", operation = "subscribe_signals", dependency = "redis", error_code = "redis_subscription_failed", error_category = "connectivity", retryable = true, error_class = %error, correlation_id = %body.correlation_id);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    if send_replay(&api.pool, &body.audience, tenant_id, &mut cursor, &sender, &body.correlation_id).await.is_err() { return; }
                    let mut messages = pubsub.on_message();
                    let mut authorization_check = tokio::time::interval(Duration::from_secs(15));
                    authorization_check.tick().await;
                    loop {
                        tokio::select! {
                            _ = authorization_check.tick() => {
                                if let Err(status) = api.authorize(&body.session_token, &body.csrf_token, &body.audience, tenant_id, &body.correlation_id).await {
                                    tracing::warn!(event.name = "realtime.subscription.authorization.revoked", component = "realtime", operation = "reauthorize_subscription", dependency = "identity_access", error_code = %status.code(), error_category = "authorization", retryable = false, correlation_id = %body.correlation_id);
                                    if sender.send(Err(Status::unauthenticated("realtime session is no longer authorized"))).await.is_err() { return; }
                                    return;
                                }
                            }
                            signal = messages.next() => match signal {
                                Some(message) => {
                        match message.get_payload::<String>() {
                            Ok(signal) if signal == "wake" => {}
                            Ok(_) => tracing::warn!(event.name = "realtime.redis.signal.invalid", component = "realtime", operation = "read_fanout_signal", dependency = "redis", error_code = "realtime_signal_invalid", error_category = "validation", retryable = false, correlation_id = %body.correlation_id),
                            Err(error) => tracing::warn!(event.name = "realtime.redis.signal.unreadable", component = "realtime", operation = "read_fanout_signal", dependency = "redis", error_code = "realtime_signal_unreadable", error_category = "validation", retryable = false, correlation_id = %body.correlation_id, error_class = %error),
                        }
                        if send_replay(&api.pool, &body.audience, tenant_id, &mut cursor, &sender, &body.correlation_id).await.is_err() { return; }
                                }
                                None => break,
                            }
                        }
                    }
                    tracing::warn!(event.name = "realtime.redis.subscription.reconnecting", component = "realtime", operation = "reconnect_pubsub", dependency = "redis", error_code = "redis_subscription_closed", error_category = "connectivity", retryable = true, correlation_id = %body.correlation_id);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }.instrument(stream_span).await;
        }.instrument(span));
        let stream = ReceiverStream::new(receiver);
        Ok(Response::new(Box::pin(stream)))
    }
}

pub async fn persist_and_publish(
    pool: &PgPool,
    redis: &mut ConnectionManager,
    envelope: &[u8],
) -> Result<bool, JournalError> {
    let event: persistence::EventEnvelope =
        serde_json::from_slice(envelope).map_err(|_| JournalError::InvalidEnvelope)?;
    let inserted = persistence::persist_event(pool, &event).await?;
    redis::cmd("PUBLISH")
        .arg(SIGNAL_CHANNEL)
        .arg("wake")
        .query_async::<i64>(redis)
        .await
        .map_err(|_| JournalError::Redis)?;
    Ok(inserted)
}

async fn send_replay(
    pool: &PgPool,
    audience: &str,
    tenant_id: Option<Uuid>,
    cursor: &mut i64,
    sender: &mpsc::Sender<Result<RealtimeEvent, Status>>,
    correlation_id: &str,
) -> Result<(), ()> {
    loop {
        let started = std::time::Instant::now();
        let mut attempt = 0_u32;
        let events = loop {
            match persistence::replay(pool, audience, tenant_id, *cursor, BATCH_SIZE).await {
                Ok(events) => break events,
                Err(error @ JournalError::Database(_)) if attempt < 2 => {
                    attempt += 1;
                    tracing::warn!(event.name = "realtime.journal.replay.retry", component = "realtime", operation = "replay_journal", dependency = "postgresql", retryable = true, attempt, correlation_id = %correlation_id);
                    tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
                    drop(error);
                }
                Err(error) => {
                    let error_code = match &error {
                        JournalError::InvalidEnvelope => "realtime_scope_invalid".to_owned(),
                        JournalError::Database(database_error) => database_error
                            .as_database_error()
                            .and_then(|cause| cause.code())
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "database_error".to_owned()),
                        JournalError::Redis => "realtime_signal_unavailable".to_owned(),
                        JournalError::InvalidPersistedEvent => "realtime_event_corrupt".to_owned(),
                        JournalError::CursorOutOfRange => "realtime_cursor_out_of_range".to_owned(),
                        JournalError::CursorGap => "realtime_cursor_gap".to_owned(),
                    };
                    let retryable = matches!(&error, JournalError::Database(_) | JournalError::Redis);
                    tracing::error!(event.name = "realtime.journal.replay.failed", component = "realtime", operation = "replay_journal", dependency = "postgresql", error_code, error_category = "persistence", retryable, dependency_duration_ms = started.elapsed().as_millis() as u64, correlation_id = %correlation_id);
                    let _ = sender.send(Err(map_journal_error(error))).await;
                    return Err(());
                }
            }
        };
        if events.is_empty() {
            return Ok(());
        }
        let has_more = events.len() == BATCH_SIZE as usize;
        for event in events {
            *cursor = event.cursor;
            sender.send(Ok(to_proto(event))).await.map_err(|_| ())?;
        }
        if !has_more {
            return Ok(());
        }
    }
}

fn parse_scope(audience: &str, tenant_id: &str) -> Result<Option<Uuid>, Status> {
    match audience {
        "platform" if tenant_id.is_empty() => Ok(None),
        "tenant" => Uuid::parse_str(tenant_id)
            .map(Some)
            .map_err(|_| Status::invalid_argument("tenant scope is invalid")),
        _ => Err(Status::invalid_argument("realtime audience is invalid")),
    }
}

fn to_proto(event: persistence::JournalEvent) -> RealtimeEvent {
    RealtimeEvent {
        cursor: event.cursor as u64,
        event_id: event.event_id.to_string(),
        event_type: event.event_type,
        aggregate_id: event.aggregate_id.to_string(),
        aggregate_version: event.aggregate_version as u64,
        audience: event.audience,
        tenant_id: event
            .tenant_id
            .map_or_else(String::new, |tenant_id| tenant_id.to_string()),
        correlation_id: event.correlation_id,
        payload_json: event.payload.to_string(),
    }
}

fn map_journal_error(error: JournalError) -> Status {
    match error {
        JournalError::InvalidEnvelope => Status::invalid_argument("realtime request is invalid"),
        JournalError::Database(_) => Status::internal("realtime journal operation failed"),
        JournalError::Redis => Status::unavailable("realtime signal is temporarily unavailable"),
        JournalError::InvalidPersistedEvent => {
            Status::internal("realtime journal operation failed")
        }
        JournalError::CursorOutOfRange => {
            Status::out_of_range("realtime cursor is outside the retained scope")
        }
        JournalError::CursorGap => {
            Status::failed_precondition("realtime journal contains a cursor gap; restart replay")
        }
    }
}

fn request_span<T>(
    request: &Request<T>,
    operation: &'static str,
    correlation_id: &str,
) -> tracing::Span {
    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&MetadataExtractor(request.metadata()))
    });
    let trace_id = parent.span().span_context().trace_id().to_string();
    let span = tracing::info_span!("grpc.module.realtime", operation, correlation_id = %correlation_id, trace_id = %trace_id);
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(event.name = "realtime.trace.parent.failed", component = "realtime", operation, error_code = "trace_parent_rejected", error_category = "observability", correlation_id = %correlation_id, error_class = %error);
    }
    span
}

struct MetadataExtractor<'a>(&'a tonic::metadata::MetadataMap);
impl opentelemetry::propagation::Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|key| match key {
                tonic::metadata::KeyRef::Ascii(key) => Some(key.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}
