use platform_contracts::payments::{
    CreatePaymentAttemptRequest, CreatePaymentAttemptResponse, ListPaymentAttemptsRequest,
    ListPaymentAttemptsResponse, PaymentAttempt as PaymentAttemptMessage,
};
use tonic::{Request, Response, Status};
use tracing::Instrument;

use super::super::{
    PaymentsApi, grpc_span, map_error, parse_uuid,
};
use crate::application::payment_service::CreatePaymentAttemptCommand;

pub async fn create(
    api: &PaymentsApi,
    request: Request<CreatePaymentAttemptRequest>,
) -> Result<Response<CreatePaymentAttemptResponse>, Status> {
    let correlation = request.get_ref().correlation_id.clone();
    let span = grpc_span(&request, "create_payment_attempt", &correlation);
    let body = request.into_inner();
    let tenant_id = parse_uuid(&body.tenant_id)?;
    let payment_intent_id = parse_uuid(&body.payment_intent_id)?;
    async move {
        let actor_key_id = api.authorize_key(
            body.api_key_secret,
            tenant_id,
            body.environment.clone(),
            "payment_attempts.create",
            correlation.clone(),
        ).await?;
        let (attempt, replayed) = api.service.create_payment_attempt(CreatePaymentAttemptCommand {
            tenant_id,
            actor_key_id,
            environment: body.environment,
            payment_intent_id,
            provider_code: body.provider_code,
            provider_account_reference: body.provider_account_reference,
            idempotency_key: body.idempotency_key,
            correlation_id: correlation,
        }).await.map_err(map_error)?;
        Ok(Response::new(CreatePaymentAttemptResponse {
            attempt: Some(attempt_message(attempt)),
            replayed,
        }))
    }.instrument(span).await
}

pub async fn list(
    api: &PaymentsApi,
    request: Request<ListPaymentAttemptsRequest>,
) -> Result<Response<ListPaymentAttemptsResponse>, Status> {
    let correlation = request.get_ref().correlation_id.clone();
    let span = grpc_span(&request, "list_payment_attempts", &correlation);
    let body = request.into_inner();
    let tenant_id = parse_uuid(&body.tenant_id)?;
    let payment_intent_id = parse_uuid(&body.payment_intent_id)?;
    let limit = if body.limit == 0 { 50 } else { body.limit };
    async move {
        let actor_key_id = api.authorize_key(
            body.api_key_secret,
            tenant_id,
            body.environment.clone(),
            "payment_attempts.read",
            correlation.clone(),
        ).await?;
        let attempts = api.service.list_payment_attempts(
            tenant_id, &body.environment, payment_intent_id,
            actor_key_id, &correlation, limit,
        ).await.map_err(map_error)?;
        Ok(Response::new(ListPaymentAttemptsResponse {
            attempts: attempts.into_iter().map(attempt_message).collect(),
        }))
    }.instrument(span).await
}

fn attempt_message(
    attempt: crate::application::payment_service::PaymentAttemptRecord,
) -> PaymentAttemptMessage {
    PaymentAttemptMessage {
        attempt_id: attempt.id.to_string(),
        tenant_id: attempt.tenant_id.to_string(),
        environment: attempt.environment,
        payment_intent_id: attempt.payment_intent_id.to_string(),
        attempt_number: attempt.attempt_number,
        provider_code: attempt.provider_code,
        provider_account_reference: attempt.provider_account_reference,
        status: attempt.status,
        aggregate_version: attempt.aggregate_version,
        created_at: attempt.created_at.to_rfc3339(),
        updated_at: attempt.updated_at.to_rfc3339(),
    }
}
