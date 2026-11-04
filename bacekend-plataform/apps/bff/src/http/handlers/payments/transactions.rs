use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{bearer_key, payment_error, payment_grpc_error, payment_request};
use crate::http::routes::AppState;
use platform_contracts::payments::ListPaymentTransactionsRequest;

#[derive(Serialize, ToSchema)]
pub struct PaymentTransactionView {
    transaction_id: String,
    tenant_id: String,
    environment: String,
    payment_intent_id: String,
    payment_attempt_id: String,
    provider_code: String,
    provider_account_reference: String,
    provider_transaction_reference: String,
    amount_minor: i64,
    currency: String,
    status: String,
    aggregate_version: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PaymentTransactionList {
    transactions: Vec<PaymentTransactionView>,
}

#[utoipa::path(get, path="/v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/transactions", params(("tenant_id" = String, Path), ("payment_intent_id" = String, Path)), responses((status=200, body=PaymentTransactionList), (status=401), (status=403), (status=404)))]
pub async fn list(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, payment_intent_id)): Path<(Uuid, Uuid)>,
    headers: axum::http::HeaderMap,
) -> Response {
    let correlation = crate::http::middleware::correlation_id::correlation_id(&headers);
    let Some((secret, environment)) = bearer_key(&headers) else {
        return payment_error(
            StatusCode::UNAUTHORIZED,
            "api_key_invalid",
            "authentication",
            false,
            correlation,
        );
    };
    let Some(mut client) = state.payments_client.clone() else {
        return payment_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "payments_unavailable",
            "dependency",
            true,
            correlation,
        );
    };
    let request = ListPaymentTransactionsRequest {
        api_key_secret: secret,
        tenant_id: tenant_id.to_string(),
        environment,
        payment_intent_id: payment_intent_id.to_string(),
        limit: 50,
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match client
        .list_payment_transactions(payment_request(request, &correlation))
        .await
    {
        Ok(response) => Json(PaymentTransactionList {
            transactions: response
                .into_inner()
                .transactions
                .into_iter()
                .map(transaction_view)
                .collect(),
        })
        .into_response(),
        Err(status) => payment_grpc_error(
            status,
            "list_payment_transactions",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

fn transaction_view(
    transaction: platform_contracts::payments::PaymentTransaction,
) -> PaymentTransactionView {
    PaymentTransactionView {
        transaction_id: transaction.transaction_id,
        tenant_id: transaction.tenant_id,
        environment: transaction.environment,
        payment_intent_id: transaction.payment_intent_id,
        payment_attempt_id: transaction.payment_attempt_id,
        provider_code: transaction.provider_code,
        provider_account_reference: transaction.provider_account_reference,
        provider_transaction_reference: transaction.provider_transaction_reference,
        amount_minor: transaction.amount_minor,
        currency: transaction.currency,
        status: transaction.status,
        aggregate_version: transaction.aggregate_version,
        created_at: transaction.created_at,
        updated_at: transaction.updated_at,
    }
}
