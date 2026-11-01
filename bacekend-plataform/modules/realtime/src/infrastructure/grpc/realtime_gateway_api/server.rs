use std::{pin::Pin, time::Duration};

use platform_contracts::realtime::{
    RealtimeEvent, ReplayEventsRequest, ReplayEventsResponse, SubscribeEventsRequest,
    realtime_gateway_server::RealtimeGateway,
};
use tokio::sync::mpsc;
use tokio_stream::{Stream, wrappers::ReceiverStream};
use tonic::{Request, Response, Status};
use tracing::Instrument;

use crate::infrastructure::persistence;
use super::{
    BATCH_SIZE, MAX_REPLAY_LIMIT, SIGNAL_CHANNEL, RealtimeGatewayApi,
    support::{map_journal_error, parse_scope, request_span, send_replay, to_proto},
};

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
