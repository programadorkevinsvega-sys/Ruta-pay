use payvia_observability::{current_trace_id, valid_correlation_id};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tracing::Instrument;

use super::{WompiEnvironment, WompiPaymentStatus};

pub struct WompiClient {
    http: reqwest::Client,
    environment: WompiEnvironment,
    private_key: String,
    base_url: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WompiTransaction {
    pub id: String,
    pub reference: String,
    pub status: WompiPaymentStatus,
    pub amount_minor: i64,
    pub currency: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WompiClientError {
    #[error("Wompi private key does not match the selected environment")]
    InvalidCredential,
    #[error("Wompi transaction identifier is invalid")]
    InvalidTransactionId,
    #[error("Wompi request timed out or the provider is unavailable")]
    Unavailable,
    #[error("Wompi rejected the lookup request")]
    Rejected,
    #[error("Wompi returned an invalid transaction representation")]
    InvalidResponse,
}

#[derive(Deserialize)]
struct TransactionEnvelope {
    data: TransactionData,
}

#[derive(Deserialize)]
struct TransactionData {
    id: String,
    reference: String,
    status: String,
    amount_in_cents: i64,
    currency: String,
}

impl WompiClient {
    pub fn new(
        environment: WompiEnvironment,
        private_key: String,
    ) -> Result<Self, WompiClientError> {
        let expected_prefix = match environment {
            WompiEnvironment::Test => "prv_test_",
            WompiEnvironment::Live => "prv_prod_",
        };
        if !private_key.starts_with(expected_prefix) || private_key.len() < 24 {
            return Err(WompiClientError::InvalidCredential);
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| WompiClientError::Unavailable)?;
        let base_url = match environment {
            WompiEnvironment::Test => "https://sandbox.wompi.co/v1",
            WompiEnvironment::Live => "https://production.wompi.co/v1",
        };
        Ok(Self {
            http,
            environment,
            private_key,
            base_url,
        })
    }

    pub async fn lookup_transaction(
        &self,
        transaction_id: &str,
        correlation_id: &str,
    ) -> Result<WompiTransaction, WompiClientError> {
        let correlation_id = if valid_correlation_id(correlation_id) {
            correlation_id.to_owned()
        } else {
            payvia_observability::new_correlation_id()
        };
        let span = tracing::info_span!(
            "gateway.provider.request",
            component = "gateway_wompi",
            operation = "lookup_transaction",
            dependency = "wompi",
            correlation_id = %correlation_id
        );
        async {
            let started_at = Instant::now();
            tracing::info!(
                event.name = "gateway.provider.request.started",
                component = "gateway_wompi",
                operation = "lookup_transaction",
                dependency = "wompi",
                correlation_id = %correlation_id,
                trace_id = %current_trace_id()
            );
            let result = self.lookup_transaction_inner(transaction_id).await;
            let duration = started_at.elapsed();
            let duration_ms = duration.as_millis() as u64;
            metrics::histogram!(
                "payvia_provider_request_duration_seconds",
                "provider" => "wompi",
                "operation" => "lookup_transaction"
            )
            .record(duration.as_secs_f64());
            match &result {
                Ok(_) => {
                    metrics::counter!(
                        "payvia_provider_requests_total",
                        "provider" => "wompi",
                        "operation" => "lookup_transaction",
                        "result" => "success"
                    )
                    .increment(1);
                    tracing::info!(
                        event.name = "gateway.provider.request.completed",
                        component = "gateway_wompi",
                        operation = "lookup_transaction",
                        dependency = "wompi",
                        result = "success",
                        duration_ms,
                        correlation_id = %correlation_id,
                        trace_id = %current_trace_id()
                    );
                }
                Err(error) => {
                    let (error_code, error_category, retryable) = match error {
                        WompiClientError::InvalidCredential => {
                            ("provider_credential_invalid", "authentication", false)
                        }
                        WompiClientError::InvalidTransactionId => {
                            ("provider_transaction_id_invalid", "validation", false)
                        }
                        WompiClientError::Unavailable => {
                            ("provider_unavailable", "connectivity", true)
                        }
                        WompiClientError::Rejected => {
                            ("provider_lookup_rejected", "provider", false)
                        }
                        WompiClientError::InvalidResponse => {
                            ("provider_response_invalid", "provider", false)
                        }
                    };
                    metrics::counter!(
                        "payvia_provider_requests_total",
                        "provider" => "wompi",
                        "operation" => "lookup_transaction",
                        "result" => "error"
                    )
                    .increment(1);
                    tracing::error!(
                        event.name = "gateway.provider.request.failed",
                        component = "gateway_wompi",
                        operation = "lookup_transaction",
                        dependency = "wompi",
                        error_code,
                        error_category,
                        retryable,
                        error_class = %error,
                        duration_ms,
                        correlation_id = %correlation_id,
                        trace_id = %current_trace_id()
                    );
                    tracing::info!(
                        event.name = "gateway.provider.request.completed",
                        component = "gateway_wompi",
                        operation = "lookup_transaction",
                        dependency = "wompi",
                        error_code,
                        error_category,
                        result = "failed",
                        duration_ms,
                        correlation_id = %correlation_id,
                        trace_id = %current_trace_id()
                    );
                }
            }
            result
        }
        .instrument(span)
        .await
    }

    async fn lookup_transaction_inner(
        &self,
        transaction_id: &str,
    ) -> Result<WompiTransaction, WompiClientError> {
        if transaction_id.is_empty()
            || transaction_id.len() > 160
            || !transaction_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(WompiClientError::InvalidTransactionId);
        }
        let url = format!("{}/transactions/{}", self.base_url, transaction_id);
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.private_key)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| WompiClientError::Unavailable)?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::NOT_FOUND
        {
            return Err(WompiClientError::Rejected);
        }
        if !status.is_success() {
            return Err(WompiClientError::Unavailable);
        }
        let body = response
            .bytes()
            .await
            .map_err(|_| WompiClientError::Unavailable)?;
        let envelope: TransactionEnvelope =
            serde_json::from_slice(&body).map_err(|_| WompiClientError::InvalidResponse)?;
        if envelope.data.id != transaction_id
            || envelope.data.amount_in_cents <= 0
            || envelope.data.currency.len() != 3
            || !envelope
                .data
                .currency
                .bytes()
                .all(|byte| byte.is_ascii_uppercase())
        {
            return Err(WompiClientError::InvalidResponse);
        }
        let status = match envelope.data.status.as_str() {
            "PENDING" => WompiPaymentStatus::Processing,
            "APPROVED" => WompiPaymentStatus::Succeeded,
            "DECLINED" | "VOIDED" | "ERROR" => WompiPaymentStatus::Failed,
            _ => return Err(WompiClientError::InvalidResponse),
        };
        Ok(WompiTransaction {
            id: envelope.data.id,
            reference: envelope.data.reference,
            status,
            amount_minor: envelope.data.amount_in_cents,
            currency: envelope.data.currency,
        })
    }
}

impl std::fmt::Debug for WompiClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WompiClient")
            .field("environment", &self.environment)
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}
