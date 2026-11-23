use crate::module_public_api::RuntimeProbeApi;
use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
use payvia_observability::{
    current_span_id, current_trace_id, new_correlation_id, valid_correlation_id,
};
use platform_catalog_pricing::{
    application::catalog_service::CatalogService,
    infrastructure::grpc::catalog_api::CatalogPricingApi,
};
use platform_contracts::identity_access::identity_access_client::IdentityAccessClient as IdentityAccessGrpcClient;
use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient,
};
use platform_contracts::{
    catalog_pricing::catalog_pricing_server::CatalogPricingServer,
    entitlements_quotas::entitlements_quotas_server::EntitlementsQuotasServer,
    identity_access::identity_access_server::IdentityAccessServer,
    payments::payments_server::PaymentsServer,
    realtime::realtime_gateway_server::RealtimeGatewayServer,
    runtime::runtime_probe_server::RuntimeProbeServer,
    tenant_registry::tenant_registry_server::TenantRegistryServer,
};
use platform_entitlements_quotas::{
    application::entitlement_service::EntitlementService,
    infrastructure::grpc::entitlements_api::EntitlementsQuotasApi,
};
use platform_identity_access::{
    application::identity_service::IdentityService,
    infrastructure::grpc::identity_access_api::IdentityAccessApi,
};
use platform_payments::{
    application::payment_service::PaymentService, infrastructure::grpc::payments_api::PaymentsApi,
};
use platform_realtime::infrastructure::grpc::realtime_gateway_api::RealtimeGatewayApi;
use platform_tenant_registry::{
    application::tenant_service::TenantService,
    infrastructure::grpc::{
        identity_access_adapter::IdentityAccessGrpcAdapter, tenant_registry_api::TenantRegistryApi,
    },
};
use redis::aio::ConnectionManager;
use std::{future::Future, sync::Arc};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub struct RuntimeModuleDependencies {
    pub pool: sqlx::PgPool,
    pub redis: ConnectionManager,
    pub redis_client: redis::Client,
    pub identity_client: IdentityAccessGrpcClient<tonic::transport::Channel>,
    pub catalog_client: CatalogPricingClient<tonic::transport::Channel>,
    pub tenant_client: TenantRegistryClient<tonic::transport::Channel>,
}

pub async fn serve<F>(
    listener: TcpListener,
    dependencies: RuntimeModuleDependencies,
    shutdown: F,
) -> Result<(), tonic::transport::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    let RuntimeModuleDependencies {
        pool,
        redis,
        redis_client,
        identity_client,
        catalog_client,
        tenant_client,
    } = dependencies;
    let identity_port = Arc::new(IdentityAccessGrpcAdapter::new(identity_client.clone()));
    let tenant_service = TenantService::new(pool.clone(), identity_port);
    let catalog_api =
        CatalogPricingApi::new(CatalogService::new(pool.clone()), identity_client.clone());
    let entitlements_api = EntitlementsQuotasApi::new(
        EntitlementService::new(pool.clone()),
        identity_client.clone(),
        catalog_client,
        tenant_client,
    );
    let payments_api = PaymentsApi::new(PaymentService::new(pool.clone()), identity_client.clone());
    Server::builder()
        .trace_fn(grpc_server_span)
        .add_service(RuntimeProbeServer::new(RuntimeProbeApi))
        .add_service(IdentityAccessServer::new(IdentityAccessApi::new(
            IdentityService::new(pool.clone(), redis),
        )))
        .add_service(TenantRegistryServer::new(TenantRegistryApi::new(
            tenant_service,
        )))
        .add_service(CatalogPricingServer::new(catalog_api))
        .add_service(EntitlementsQuotasServer::new(entitlements_api))
        .add_service(PaymentsServer::new(payments_api))
        .add_service(RealtimeGatewayServer::new(RealtimeGatewayApi::new(
            pool,
            redis_client,
            identity_client,
        )))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
}

fn grpc_server_span(request: &tonic::codegen::http::Request<()>) -> tracing::Span {
    let correlation_id = request
        .headers()
        .get("x-correlation-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_correlation_id(value))
        .map(str::to_owned)
        .unwrap_or_else(new_correlation_id);
    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&GrpcHeaderExtractor(request.headers()))
    });
    let trace_context_valid = parent.span().span_context().is_valid();
    let span = tracing::info_span!(
        "grpc.server.request",
        rpc.method = %request.uri().path(),
        correlation_id = %correlation_id,
        trace_id = tracing::field::Empty,
        span_id = tracing::field::Empty
    );
    if let Err(error) = span.set_parent(parent) {
        tracing::warn!(
            event.name = "grpc.trace_context.rejected",
            component = "platform_runtime",
            operation = "server_request",
            error_code = %error,
            correlation_id = %correlation_id
        );
    }
    let (trace_id, span_id) = span.in_scope(|| (current_trace_id(), current_span_id()));
    span.record("trace_id", tracing::field::display(&trace_id));
    span.record("span_id", tracing::field::display(&span_id));
    span.in_scope(|| {
        if request.headers().contains_key("traceparent") && !trace_context_valid {
            tracing::warn!(
                event.name = "grpc.trace_context.rejected",
                component = "platform_runtime",
                operation = "server_request",
                error_code = "traceparent_invalid",
                correlation_id = %correlation_id,
                trace_id = %trace_id,
                span_id = %span_id
            );
        }
        tracing::info!(
            event.name = "grpc.request.started",
            component = "platform_runtime",
            operation = %request.uri().path(),
            correlation_id = %correlation_id,
            trace_id = %trace_id,
            span_id = %span_id
        );
    });
    span
}

struct GrpcHeaderExtractor<'a>(&'a tonic::codegen::http::HeaderMap);

impl Extractor for GrpcHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|key| key.as_str()).collect()
    }
}
