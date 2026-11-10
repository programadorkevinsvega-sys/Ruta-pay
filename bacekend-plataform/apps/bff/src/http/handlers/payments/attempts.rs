use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{bearer_key, payment_error, payment_grpc_error, payment_request};
use crate::http::routes::AppState;
use platform_contracts::payments::{CreatePaymentAttemptRequest, ListPaymentAttemptsRequest};

#[derive(Deserialize, ToSchema)]
pub struct NewPaymentAttempt {
    provider_code: String,
    provider_account_reference: String,
}

#[derive(Serialize, ToSchema)]
pub struct PaymentAttemptView {
    attempt_id: String,
    tenant_id: String,
    environment: String,
    payment_intent_id: String,
    attempt_number: i32,
    provider_code: String,
    provider_account_reference: String,
    status: String,
    aggregate_version: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PaymentAttemptList {
    attempts: Vec<PaymentAttemptView>,
}

#[utoipa::path(post, path="/v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/attempts", request_body=NewPaymentAttempt, responses((status=201, body=PaymentAttemptView), (status=200, body=PaymentAttemptView), (status=400), (status=401), (status=403), (status=404), (status=409)))]
pub async fn create(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, payment_intent_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<NewPaymentAttempt>,
) -> Response {
    let correlation = crate::http::middleware::correlation_id::correlation_id(&headers);
    let tenant_id = match Uuid::parse_str(&tenant_id) {
        Ok(id) => id,
        Err(_) => {
            return payment_error(
                StatusCode::BAD_REQUEST,
                "payment_identifier_invalid",
                "validation",
                false,
                correlation,
            );
        }
    };
    let payment_intent_id = match Uuid::parse_str(&payment_intent_id) {
        Ok(id) => id,
        Err(_) => {
            return payment_error(
                StatusCode::BAD_REQUEST,
                "payment_identifier_invalid",
                "validation",
                false,
                correlation,
            );
        }
    };
    let Some((secret, environment)) = bearer_key(&headers) else {
        return payment_error(
            StatusCode::UNAUTHORIZED,
            "api_key_invalid",
            "authentication",
            false,
            correlation,
        );
    };
    let Some(idempotency_key) = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
    else {
        return payment_error(
            StatusCode::BAD_REQUEST,
            "idempotency_key_required",
            "validation",
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
    let request = CreatePaymentAttemptRequest {
        api_key_secret: secret,
        tenant_id: tenant_id.to_string(),
        environment,
        payment_intent_id: payment_intent_id.to_string(),
        provider_code: body.provider_code,
        provider_account_reference: body.provider_account_reference,
        idempotency_key: idempotency_key.to_owned(),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match client
        .create_payment_attempt(payment_request(request, &correlation))
        .await
    {
        Ok(response) => {
            let response = response.into_inner();
            let Some(attempt) = response.attempt else {
                return payment_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "payment_response_invalid",
                    "internal",
                    false,
                    correlation,
                );
            };
            let view = attempt_view(attempt);
            (
                if response.replayed {
                    StatusCode::OK
                } else {
                    StatusCode::CREATED
                },
                Json(view),
            )
                .into_response()
        }
        Err(status) => payment_grpc_error(
            status,
            "create_payment_attempt",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path="/v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/attempts", params(("tenant_id" = String, Path), ("payment_intent_id" = String, Path)), responses((status=200, body=PaymentAttemptList), (status=401), (status=403), (status=404)))]
pub async fn list(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, payment_intent_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
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
    let request = ListPaymentAttemptsRequest {
        api_key_secret: secret,
        tenant_id: tenant_id.to_string(),
        environment,
        payment_intent_id: payment_intent_id.to_string(),
        limit: 50,
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match client
        .list_payment_attempts(payment_request(request, &correlation))
        .await
    {
        Ok(response) => Json(PaymentAttemptList {
            attempts: response
                .into_inner()
                .attempts
                .into_iter()
                .map(attempt_view)
                .collect(),
        })
        .into_response(),
        Err(status) => payment_grpc_error(
            status,
            "list_payment_attempts",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

fn attempt_view(attempt: platform_contracts::payments::PaymentAttempt) -> PaymentAttemptView {
    PaymentAttemptView {
        attempt_id: attempt.attempt_id,
        tenant_id: attempt.tenant_id,
        environment: attempt.environment,
        payment_intent_id: attempt.payment_intent_id,
        attempt_number: attempt.attempt_number,
        provider_code: attempt.provider_code,
        provider_account_reference: attempt.provider_account_reference,
        status: attempt.status,
        aggregate_version: attempt.aggregate_version,
        created_at: attempt.created_at,
        updated_at: attempt.updated_at,
    }
}
