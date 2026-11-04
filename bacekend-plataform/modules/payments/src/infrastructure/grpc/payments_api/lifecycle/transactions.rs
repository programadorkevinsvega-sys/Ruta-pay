use platform_contracts::payments::{
    ListPaymentTransactionsRequest, ListPaymentTransactionsResponse,
    PaymentTransaction as PaymentTransactionMessage,
};
use tonic::{Request, Response, Status};
use tracing::Instrument;

use super::super::{PaymentsApi, grpc_span, map_error, parse_uuid};

pub async fn list(
    api: &PaymentsApi,
    request: Request<ListPaymentTransactionsRequest>,
) -> Result<Response<ListPaymentTransactionsResponse>, Status> {
    let correlation = request.get_ref().correlation_id.clone();
    let span = grpc_span(&request, "list_payment_transactions", &correlation);
    let body = request.into_inner();
    let tenant_id = parse_uuid(&body.tenant_id)?;
    let payment_intent_id = parse_uuid(&body.payment_intent_id)?;
    let limit = if body.limit == 0 { 50 } else { body.limit };
    async move {
        let actor_key_id = api.authorize_key(
            body.api_key_secret,
            tenant_id,
            body.environment.clone(),
            "payment_transactions.read",
            correlation.clone(),
        ).await?;
        let transactions = api.service.list_payment_transactions(
            tenant_id, &body.environment, payment_intent_id,
            actor_key_id, &correlation, limit,
        ).await.map_err(map_error)?;
        Ok(Response::new(ListPaymentTransactionsResponse {
            transactions: transactions.into_iter().map(transaction_message).collect(),
        }))
    }.instrument(span).await
}

fn transaction_message(
    transaction: crate::application::payment_service::PaymentTransactionRecord,
) -> PaymentTransactionMessage {
    PaymentTransactionMessage {
        transaction_id: transaction.id.to_string(),
        tenant_id: transaction.tenant_id.to_string(),
        environment: transaction.environment,
        payment_intent_id: transaction.payment_intent_id.to_string(),
        payment_attempt_id: transaction.payment_attempt_id.to_string(),
        provider_code: transaction.provider_code,
        provider_account_reference: transaction.provider_account_reference,
        provider_transaction_reference: transaction.provider_transaction_reference,
        amount_minor: transaction.amount_minor,
        currency: transaction.currency,
        status: transaction.status,
        aggregate_version: transaction.aggregate_version,
        created_at: transaction.created_at.to_rfc3339(),
        updated_at: transaction.updated_at.to_rfc3339(),
    }
}
