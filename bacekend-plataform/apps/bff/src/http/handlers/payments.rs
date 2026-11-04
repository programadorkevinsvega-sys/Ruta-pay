pub mod attempts;
pub mod status;
pub mod transactions;

pub use attempts::{create as create_payment_attempt, list as list_payment_attempts};
pub use status::update as update_payment_intent_status;
pub use transactions::list as list_payment_transactions;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::http::{
    dto::error_envelope::{ErrorBody, ErrorEnvelope},
    middleware::correlation_id::correlation_id,
    routes::AppState,
};
use payvia_observability::{current_span_id, current_trace_id};
use platform_contracts::payments::{CreatePaymentIntentRequest, GetPaymentIntentRequest};

#[derive(Deserialize, ToSchema)]
pub struct NewPaymentIntent {
    amount_minor: i64,
    currency: String,
    payment_method_type: String,
    merchant_reference: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PaymentIntentView {
    payment_intent_id: String,
    tenant_id: String,
    environment: String,
    amount_minor: i64,
    currency: String,
    payment_method_type: String,
    merchant_reference: Option<String>,
    status: String,
    created_at: String,
}

#[utoipa::path(post, path="/v1/tenants/{tenant_id}/payment-intents", request_body=NewPaymentIntent, responses((status=201, body=PaymentIntentView), (status=200, body=PaymentIntentView), (status=400), (status=401), (status=403), (status=409)))]
pub async fn create_payment_intent(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<NewPaymentIntent>,
) -> Response {
    let correlation = correlation_id(&headers);
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
    let request = CreatePaymentIntentRequest {
        api_key_secret: secret,
        tenant_id: tenant_id.to_string(),
        environment,
        amount_minor: body.amount_minor,
        currency: body.currency,
        payment_method_type: body.payment_method_type,
        merchant_reference: body.merchant_reference.unwrap_or_default(),
        idempotency_key: idempotency_key.to_owned(),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match client
        .create_payment_intent(payment_request(request, &correlation))
        .await
    {
        Ok(response) => {
            let response = response.into_inner();
            let Some(intent) = response.payment_intent else {
                return payment_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "payment_response_invalid",
                    "internal",
                    false,
                    correlation,
                );
            };
            let status = if response.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(payment_view(intent))).into_response()
        }
        Err(status) => payment_grpc_error(
            status,
            "create_payment_intent",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path="/v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}", params(("tenant_id" = String, Path), ("payment_intent_id" = String, Path)), responses((status=200, body=PaymentIntentView), (status=401), (status=403), (status=404)))]
pub async fn get_payment_intent(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, payment_intent_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
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
    let request = GetPaymentIntentRequest {
        api_key_secret: secret,
        tenant_id: tenant_id.to_string(),
        environment,
        payment_intent_id: payment_intent_id.to_string(),
        correlation_id: correlation.clone(),
    };
    let started = std::time::Instant::now();
    match client
        .get_payment_intent(payment_request(request, &correlation))
        .await
    {
        Ok(response) => match response.into_inner().payment_intent {
            Some(intent) => Json(payment_view(intent)).into_response(),
            None => payment_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "payment_response_invalid",
                "internal",
                false,
                correlation,
            ),
        },
        Err(status) => payment_grpc_error(
            status,
            "get_payment_intent",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

pub(super) fn payment_request<T>(body: T, correlation: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(body);
    if let Some(traceparent) = payvia_observability::current_traceparent()
        && let Ok(value) = tonic::metadata::MetadataValue::try_from(traceparent.as_str())
    {
        request.metadata_mut().insert("traceparent", value);
    }
    if let Ok(value) = tonic::metadata::MetadataValue::try_from(correlation) {
        request.metadata_mut().insert("x-correlation-id", value);
    }
    request.set_timeout(Duration::from_secs(3));
    request
}

pub(super) fn bearer_key(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, secret) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || secret.is_empty()
        || secret.len() > 128
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return None;
    }
    let environment = if secret.starts_with("test_") {
        "test"
    } else if secret.starts_with("live_") {
        "live"
    } else {
        return None;
    };
    Some((secret.to_owned(), environment.to_owned()))
}

pub(super) fn payment_view(intent: platform_contracts::payments::PaymentIntent) -> PaymentIntentView {
    PaymentIntentView {
        payment_intent_id: intent.payment_intent_id,
        tenant_id: intent.tenant_id,
        environment: intent.environment,
        amount_minor: intent.amount_minor,
        currency: intent.currency,
        payment_method_type: intent.payment_method_type,
        merchant_reference: (!intent.merchant_reference.is_empty())
            .then_some(intent.merchant_reference),
        status: intent.status,
        created_at: intent.created_at,
    }
}

pub(super) fn payment_grpc_error(
    status: tonic::Status,
    operation: &'static str,
    correlation: &str,
    duration_ms: u64,
) -> Response {
    let (http, code, category, retryable) = match status.code() {
        tonic::Code::InvalidArgument => (
            StatusCode::BAD_REQUEST,
            "payment_input_invalid",
            "validation",
            false,
        ),
        tonic::Code::FailedPrecondition | tonic::Code::Aborted => (
            StatusCode::CONFLICT,
            "payment_transition_conflict",
            "conflict",
            false,
        ),
        tonic::Code::AlreadyExists => (
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "conflict",
            false,
        ),
        tonic::Code::Unauthenticated => (
            StatusCode::UNAUTHORIZED,
            "api_key_invalid",
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
            "payment_intent_not_found",
            "not_found",
            false,
        ),
        tonic::Code::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "payments_unavailable",
            "dependency",
            true,
        ),
        tonic::Code::DeadlineExceeded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "payments_timeout",
            "timeout",
            true,
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "payment_intent_failed",
            "internal",
            false,
        ),
    };
    tracing::warn!(event.name="bff.dependency.call.failed", component="platform_bff", operation, dependency="payments", error_code=%status.code(), error_category=category, retryable, dependency_duration_ms=duration_ms, correlation_id=%correlation, trace_id=%current_trace_id(), span_id=%current_span_id());
    payment_error(http, code, category, retryable, correlation.to_owned())
}

pub(super) fn payment_error(
    status: StatusCode,
    code: &'static str,
    category: &'static str,
    retryable: bool,
    correlation_id: String,
) -> Response {
    let body = ErrorEnvelope {
        error: ErrorBody {
            code,
            category,
            message: "The payment intent operation could not be completed.",
            correlation_id,
            trace_id: current_trace_id(),
            retryable,
        },
    };
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert("x-internal-error-code", HeaderValue::from_static(code));
    response.headers_mut().insert(
        "x-internal-error-category",
        HeaderValue::from_static(category),
    );
    response
}
