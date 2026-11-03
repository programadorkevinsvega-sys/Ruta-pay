use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
use platform_contracts::{
    identity_access::{
        AuthorizeApiKeyOperationRequest, identity_access_client::IdentityAccessClient,
    },
    payments::{
        CreatePaymentIntentRequest, CreatePaymentIntentResponse, GetPaymentIntentRequest,
        GetPaymentIntentResponse, PaymentIntent as PaymentIntentMessage, payments_server::Payments,
    },
};
use tonic::{Request, Response, Status};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::{
    application::payment_service::{
        CreatePaymentIntentCommand, PaymentService, PaymentServiceError,
    },
    domain::payment_intent::PaymentIntent,
};

#[derive(Clone)]
pub struct PaymentsApi {
    service: PaymentService,
    identity: IdentityAccessClient<tonic::transport::Channel>,
}

impl PaymentsApi {
    pub fn new(
        service: PaymentService,
        identity: IdentityAccessClient<tonic::transport::Channel>,
    ) -> Self {
        Self { service, identity }
    }

    async fn authorize_key(
        &self,
        secret: String,
        tenant_id: Uuid,
        environment: String,
        scope: &'static str,
        correlation_id: String,
    ) -> Result<Uuid, Status> {
        let response = self
            .identity
            .clone()
            .authorize_api_key_operation(Request::new(AuthorizeApiKeyOperationRequest {
                secret,
                tenant_id: tenant_id.to_string(),
                environment,
                required_scope: scope.to_owned(),
                correlation_id,
            }))
            .await?
            .into_inner();
        Uuid::parse_str(&response.key_id)
            .map_err(|_| Status::internal("API key identity is invalid"))
    }
}

#[tonic::async_trait]
impl Payments for PaymentsApi {
    async fn create_payment_intent(
        &self,
        request: Request<CreatePaymentIntentRequest>,
    ) -> Result<Response<CreatePaymentIntentResponse>, Status> {
        let correlation = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "create_payment_intent", &correlation);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id)?;
        async move {
            let actor_key_id = self
                .authorize_key(
                    body.api_key_secret,
                    tenant_id,
                    body.environment.clone(),
                    "payment_intents:create",
                    correlation.clone(),
                )
                .await?;
            let command = CreatePaymentIntentCommand {
                tenant_id,
                actor_key_id,
                environment: body.environment,
                amount_minor: body.amount_minor,
                currency: body.currency,
                payment_method_type: body.payment_method_type,
                merchant_reference: (!body.merchant_reference.is_empty())
                    .then_some(body.merchant_reference),
                idempotency_key: body.idempotency_key,
                correlation_id: correlation,
            };
            let (intent, replayed) = self
                .service
                .create_payment_intent(command)
                .await
                .map_err(map_error)?;
            Ok(Response::new(CreatePaymentIntentResponse {
                payment_intent: Some(payment_intent_message(intent)),
                replayed,
            }))
        }
        .instrument(span)
        .await
    }

    async fn get_payment_intent(
        &self,
        request: Request<GetPaymentIntentRequest>,
    ) -> Result<Response<GetPaymentIntentResponse>, Status> {
        let correlation = request.get_ref().correlation_id.clone();
        let span = grpc_span(&request, "get_payment_intent", &correlation);
        let body = request.into_inner();
        let tenant_id = parse_uuid(&body.tenant_id)?;
        let intent_id = parse_uuid(&body.payment_intent_id)?;
        async move {
            self.authorize_key(
                body.api_key_secret,
                tenant_id,
                body.environment,
                "payment_intents:read",
                correlation,
            )
            .await?;
            let intent = self
                .service
                .get_payment_intent(tenant_id, intent_id)
                .await
                .map_err(map_error)?;
            Ok(Response::new(GetPaymentIntentResponse {
                payment_intent: Some(payment_intent_message(intent)),
            }))
        }
        .instrument(span)
        .await
    }
}

fn payment_intent_message(intent: PaymentIntent) -> PaymentIntentMessage {
    PaymentIntentMessage {
        payment_intent_id: intent.id.to_string(),
        tenant_id: intent.tenant_id.to_string(),
        environment: intent.environment,
        amount_minor: intent.amount_minor,
        currency: intent.currency,
        payment_method_type: intent.payment_method_type.as_str().to_owned(),
        merchant_reference: intent.merchant_reference.unwrap_or_default(),
        status: intent.status,
        created_at: intent.created_at.to_rfc3339(),
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("identifier is invalid"))
}

fn map_error(error: PaymentServiceError) -> Status {
    match error {
        PaymentServiceError::Domain(
            crate::domain::payment_intent::PaymentIntentError::AmountInvalid
            | crate::domain::payment_intent::PaymentIntentError::CurrencyInvalid
            | crate::domain::payment_intent::PaymentIntentError::PaymentMethodInvalid
            | crate::domain::payment_intent::PaymentIntentError::EnvironmentInvalid
            | crate::domain::payment_intent::PaymentIntentError::IdempotencyKeyInvalid
            | crate::domain::payment_intent::PaymentIntentError::MerchantReferenceInvalid,
        ) => Status::invalid_argument("payment intent input is invalid"),
        PaymentServiceError::Domain(
            crate::domain::payment_intent::PaymentIntentError::IdempotencyConflict,
        ) => Status::already_exists("idempotency key conflicts with the original request"),
        PaymentServiceError::Domain(
            crate::domain::payment_intent::PaymentIntentError::NotFound,
        ) => Status::not_found("payment intent was not found"),
        PaymentServiceError::Database(error) => {
            tracing::error!(event.name="payment.persistence.failed",component="payments",error_class=%error);
            Status::internal("payment intent could not be completed")
        }
        PaymentServiceError::Serialization(_) | PaymentServiceError::SnapshotUnavailable => {
            tracing::error!(
                event.name = "payment.idempotency.snapshot.invalid",
                component = "payments",
                error_code = "idempotency_snapshot_invalid"
            );
            Status::internal("payment intent could not be completed")
        }
    }
}

fn grpc_span<T>(request: &Request<T>, operation: &'static str, correlation: &str) -> tracing::Span {
    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&GrpcMetadataExtractor(request.metadata()))
    });
    let span =
        tracing::info_span!("grpc.module.payments", operation, correlation_id = %correlation);
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(event.name="grpc.trace_context.rejected",component="payments",operation,error_category="trace_context",error_code=%error,correlation_id=%correlation);
    }
    span
}

struct GrpcMetadataExtractor<'a>(&'a tonic::metadata::MetadataMap);

impl Extractor for GrpcMetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|key| match key {
                tonic::metadata::KeyRef::Ascii(value) => Some(value.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}
