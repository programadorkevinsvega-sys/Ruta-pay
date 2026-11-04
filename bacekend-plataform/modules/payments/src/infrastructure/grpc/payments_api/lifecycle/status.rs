use platform_contracts::payments::{
    UpdatePaymentIntentStatusRequest, UpdatePaymentIntentStatusResponse,
};
use tonic::{Request, Response, Status};
use tracing::Instrument;

use super::super::{
    PaymentsApi, grpc_span, map_error, parse_uuid, payment_intent_message,
};
use crate::application::payment_service::UpdatePaymentIntentStatusCommand;

pub async fn update(
    api: &PaymentsApi,
    request: Request<UpdatePaymentIntentStatusRequest>,
) -> Result<Response<UpdatePaymentIntentStatusResponse>, Status> {
    let correlation = request.get_ref().correlation_id.clone();
    let span = grpc_span(&request, "update_payment_intent_status", &correlation);
    let body = request.into_inner();
    let tenant_id = parse_uuid(&body.tenant_id)?;
    let payment_intent_id = parse_uuid(&body.payment_intent_id)?;
    let payment_attempt_id = parse_uuid(&body.payment_attempt_id)?;
    async move {
        let actor_key_id = api.authorize_key(
            body.api_key_secret,
            tenant_id,
            body.environment.clone(),
            "payment_intents.update",
            correlation.clone(),
        ).await?;
        let update = api.service.update_payment_intent_status(UpdatePaymentIntentStatusCommand {
            tenant_id,
            actor_key_id,
            environment: body.environment,
            payment_intent_id,
            payment_attempt_id,
            status: body.status,
            expected_version: body.expected_version,
            correlation_id: correlation,
        }).await.map_err(map_error)?;
        Ok(Response::new(UpdatePaymentIntentStatusResponse {
            payment_intent: Some(payment_intent_message(update.intent)),
            aggregate_version: update.aggregate_version,
        }))
    }.instrument(span).await
}
