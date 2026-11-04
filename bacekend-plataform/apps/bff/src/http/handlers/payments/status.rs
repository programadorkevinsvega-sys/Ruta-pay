use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::http::routes::AppState;
use platform_contracts::payments::UpdatePaymentIntentStatusRequest;
use super::{bearer_key, payment_error, payment_grpc_error, payment_request, payment_view, PaymentIntentView};

#[derive(Deserialize, ToSchema)]
pub struct NewPaymentIntentStatus {
    payment_attempt_id: Uuid,
    status: String,
    expected_version: i64,
}

#[derive(Serialize, ToSchema)]
pub struct PaymentIntentStatusView {
    payment_intent: PaymentIntentView,
    aggregate_version: i64,
}

#[utoipa::path(patch, path="/v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/status", request_body=NewPaymentIntentStatus, responses((status=200, body=PaymentIntentStatusView), (status=400), (status=401), (status=403), (status=404), (status=409)))]
pub async fn update(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, payment_intent_id)): Path<(Uuid, Uuid)>,
    headers: axum::http::HeaderMap,
    Json(body): Json<NewPaymentIntentStatus>,
) -> Response {
    let correlation = crate::http::middleware::correlation_id::correlation_id(&headers);
    let Some((secret, environment)) = bearer_key(&headers) else {
        return payment_error(StatusCode::UNAUTHORIZED, "api_key_invalid", "authentication", false, correlation);
    };
    let Some(mut client) = state.payments_client.clone() else {
        return payment_error(StatusCode::SERVICE_UNAVAILABLE, "payments_unavailable", "dependency", true, correlation);
    };
    let request = UpdatePaymentIntentStatusRequest {
        api_key_secret: secret,
        tenant_id: tenant_id.to_string(),
        environment,
        payment_intent_id: payment_intent_id.to_string(),
        payment_attempt_id: body.payment_attempt_id.to_string(),
        status: body.status,
        expected_version: body.expected_version,
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match client.update_payment_intent_status(payment_request(request, &correlation)).await {
        Ok(response) => {
            let response = response.into_inner();
            match response.payment_intent {
                Some(intent) => Json(PaymentIntentStatusView {
                    payment_intent: payment_view(intent),
                    aggregate_version: response.aggregate_version,
                }).into_response(),
                None => payment_error(StatusCode::INTERNAL_SERVER_ERROR, "payment_response_invalid", "internal", false, correlation),
            }
        }
        Err(status) => payment_grpc_error(status, "update_payment_intent_status", &correlation, started.elapsed().as_millis() as u64),
    }
}
