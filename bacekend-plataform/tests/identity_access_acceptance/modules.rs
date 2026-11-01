pub mod api_key_acceptance_flow;
pub mod api_key_authorization;
pub mod api_key_creation;
pub mod api_key_dto_validation;
pub mod api_key_lifecycle;
pub mod concurrent_registration;
pub mod credential_validation;
pub mod database_fixture;
pub mod database_migrations;
pub mod duplicate_registration;
pub mod environment_fixture;
pub mod event_fixture;
pub mod identity_outbox_recovery;
pub mod login_rate_limits;
pub mod operator_realtime_revocation;
pub mod operator_redis_fixture;
pub mod operator_revocation_audit;
pub mod operator_role_matrix;
pub mod operator_session_acceptance;
pub mod outbox_database_failures;
pub mod outbox_leases;
pub mod outbox_transaction_rollback;
#[path = "phase3_catalog.rs"]
pub mod phase3_catalog;
#[path = "phase3_entitlements.rs"]
pub mod phase3_entitlements;
pub mod platform_realtime_replay;
pub mod platform_replay_and_logout;
pub mod rate_limit_acceptance;
pub mod rate_limit_telemetry;
pub mod realtime_acceptance_flow;
pub mod realtime_journal_acceptance;
pub mod second_tenant_flow;
pub mod second_tenant_isolation;
pub mod second_tenant_registration;
pub mod secondary_tenant_lifecycle;
pub mod server_fixture;
pub mod session_security;
pub mod session_security_flow;
pub mod tenant_admin_acceptance_flow;
pub mod tenant_dto_validation;
pub mod tenant_event_delivery;
pub mod tenant_onboarding_phase;
pub mod tenant_outbox_recovery;
pub mod tenant_platform_access;
pub mod tenant_realtime_replay;
pub mod tenant_realtime_revocation;
pub mod tenant_registration_acceptance;
pub mod tenant_registration_races;
pub mod tenant_role_matrix;
pub mod tenant_session_acceptance;
pub mod tenant_status_input_validation;
pub mod tenant_status_lifecycle;
pub mod tenant_status_outbox_acceptance;
pub mod tenant_status_race;
pub mod tenant_status_version_validation;

use async_nats::jetstream;
#[path = "support.rs"]
mod identity_access_acceptance_support;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use identity_access_acceptance_support::{
    AcceptanceDatabaseCleanup, EventCapture, EventStreamCleanup, cookie_value, is_stream_not_found,
};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use platform_contracts::{
    catalog_pricing::catalog_pricing_client::CatalogPricingClient as CatalogPricingGrpcClient,
    entitlements_quotas::entitlements_quotas_client::EntitlementsQuotasClient as EntitlementsQuotasGrpcClient,
    identity_access::{
        AuthorizeApiKeyOperationRequest, AuthorizeOperatorOperationRequest,
        AuthorizeTenantOperationRequest, identity_access_client::IdentityAccessClient,
    },
    realtime::{
        SubscribeEventsRequest,
        realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient,
    },
    runtime::runtime_probe_client::RuntimeProbeClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient,
};
use platform_identity_access::application::operator_bootstrap::OperatorBootstrapService;
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use reqwest::{Client, StatusCode, header};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, sync::oneshot};
use tracing_subscriber::prelude::*;
use uuid::Uuid;

pub use environment_fixture::IdentityEnvironment;
pub use identity_access_acceptance_support::is_stream_not_found;
pub use tenant_onboarding_phase::TenantOnboardingFixture;
