mod api_key_creation;
mod api_key_dto_validation;
mod concurrent_registration;
mod credential_validation;
mod database_migrations;
mod duplicate_registration;
mod identity_outbox_recovery;
mod operator_redis_fixture;
mod operator_session_acceptance;
mod operator_role_matrix;
mod outbox_database_failures;
mod outbox_leases;
mod outbox_transaction_rollback;
#[path = "identity_access_acceptance/phase3_catalog.rs"]
mod phase3_catalog;
#[path = "identity_access_acceptance/phase3_entitlements.rs"]
mod phase3_entitlements;
mod rate_limit_acceptance;
mod rate_limit_telemetry;
mod secondary_tenant_lifecycle;
mod session_security;
mod tenant_dto_validation;
mod tenant_event_delivery;
mod tenant_outbox_recovery;
mod tenant_platform_access;
mod tenant_registration_acceptance;
mod tenant_registration_races;
mod tenant_session_acceptance;
mod tenant_role_matrix;
mod tenant_status_input_validation;
mod tenant_status_lifecycle;
mod tenant_status_outbox_acceptance;
mod tenant_status_race;
mod tenant_status_version_validation;

use async_nats::jetstream;
#[path = "identity_access_acceptance/support.rs"]
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

#[tokio::test(flavor = "multi_thread")]
async fn identity_http_grpc_postgres_redis_cookie_csrf_rate_limit_and_cleanup() {
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    let captured = EventCapture::default();
    let captured_events = captured.0.clone();
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("identity-acceptance")))
        .with(captured)
        .try_init()
        .expect("identity acceptance subscriber initializes");

    let admin_url = std::env::var("PLATFORM_TEST_DATABASE_ADMIN_URL")
        .expect("development/admin PostgreSQL URL is configured for integration acceptance");
    let redis_url =
        std::env::var("PLATFORM_REDIS_URL").expect("real development Redis URL is configured");
    let admin_pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&admin_url)
        .await
        .expect("development PostgreSQL accepts the admin connection");
    let database_name = format!("phase2_{}", Uuid::new_v4().simple());
    let runtime_role = format!("phase2_{}", Uuid::new_v4().simple());
    let runtime_password = Uuid::new_v4().simple().to_string();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE \"{database_name}\""
    )))
    .execute(&admin_pool)
    .await
    .expect("isolated acceptance database is created");
    let mut database_cleanup = AcceptanceDatabaseCleanup {
        admin_pool: admin_pool.clone(),
        database_name: database_name.clone(),
        runtime_role: runtime_role.clone(),
        armed: true,
    };

    let mut parsed_url = url::Url::parse(&admin_url).expect("admin database URL parses");
    parsed_url.set_path(&format!("/{database_name}"));
    let migration_pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(parsed_url.as_str())
        .await
        .expect("isolated acceptance database accepts migration connection");
    database_migrations::migrate_and_verify_permission_seeds(&migration_pool).await;

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE \"{runtime_role}\" LOGIN PASSWORD '{runtime_password}'"
    )))
    .execute(&admin_pool)
    .await
    .expect("a restricted application role is created");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT CONNECT ON DATABASE \"{database_name}\" TO \"{runtime_role}\""
    )))
    .execute(&admin_pool)
    .await
    .expect("runtime role receives database connect only");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("runtime role receives schema usage");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\"")))
        .execute(&migration_pool)
        .await
        .expect("runtime role receives owned-module table privileges");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA identity_access, tenant_registry, realtime, catalog_pricing, entitlements_quotas TO \"{runtime_role}\"")))
        .execute(&migration_pool)
        .await
        .expect("runtime role receives sequence privileges for every migrated module");
    parsed_url
        .set_username(&runtime_role)
        .expect("runtime role is valid in database URL");
    parsed_url
        .set_password(Some(&runtime_password))
        .expect("runtime password is valid in database URL");
    let runtime_pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(3))
        .connect(parsed_url.as_str())
        .await
        .expect("non-superuser runtime role connects to the isolated database");
    let bypass_rls: bool =
        sqlx::query_scalar("SELECT rolbypassrls FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&runtime_pool)
            .await
            .expect("runtime RLS attribute is readable");
    assert!(
        !bypass_rls,
        "runtime identity must not bypass row-level security"
    );

    let operator_fixture = operator_redis_fixture::bootstrap_and_verify_redis_failure(
        &runtime_pool,
        &migration_pool,
        &redis_url,
    )
    .await;
    let operator_email = operator_fixture.operator_email;
    let operator_password = operator_fixture.operator_password;
    let operator_id = operator_fixture.operator_id;
    let redis_client = operator_fixture.redis_client;
    let redis_manager = operator_fixture.redis_manager;

    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.expect("gRPC bind");
    let grpc_addr = grpc_listener.local_addr().expect("gRPC address");
    let runtime_channel = tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
        .expect("identity module endpoint parses")
        .connect_lazy();
    let module_identity_client = IdentityAccessClient::new(runtime_channel.clone());
    let module_catalog_client = CatalogPricingGrpcClient::new(runtime_channel.clone());
    let module_tenant_client = TenantRegistryGrpcClient::new(runtime_channel);
    let (grpc_stop, grpc_signal) = oneshot::channel();
    let grpc_pool = runtime_pool.clone();
    let grpc_task = tokio::spawn(platform_runtime::module_config::serve(
        grpc_listener,
        platform_runtime::module_config::RuntimeModuleDependencies {
            pool: grpc_pool,
            redis: redis_manager.clone(),
            redis_client,
            identity_client: module_identity_client,
            catalog_client: module_catalog_client,
            tenant_client: module_tenant_client,
        },
        async move {
            let _ = grpc_signal.await;
        },
    ));
    let grpc_endpoint = tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
        .expect("gRPC endpoint parses");
    let runtime_client = RuntimeProbeClient::connect(grpc_endpoint.clone())
        .await
        .expect("runtime gRPC channel connects");
    let identity_client = IdentityAccessClient::connect(grpc_endpoint)
        .await
        .expect("identity gRPC channel connects");
    let tenant_client = TenantRegistryGrpcClient::connect(format!("http://{grpc_addr}"))
        .await
        .expect("tenant registry gRPC channel connects");
    let realtime_client = RealtimeGatewayGrpcClient::connect(format!("http://{grpc_addr}"))
        .await
        .expect("realtime gRPC channel connects");
    let catalog_client = CatalogPricingGrpcClient::connect(format!("http://{grpc_addr}"))
        .await
        .expect("catalog gRPC channel connects");
    let entitlements_client = EntitlementsQuotasGrpcClient::connect(format!("http://{grpc_addr}"))
        .await
        .expect("entitlements gRPC channel connects");
    let mut authorization_client = identity_client.clone();
    let mut realtime_acceptance_client = realtime_client.clone();
    let http_listener = TcpListener::bind("127.0.0.1:0").await.expect("BFF bind");
    let http_addr = http_listener.local_addr().expect("BFF address");
    let origin = format!("http://localhost:{}", http_addr.port());
    let (http_stop, http_signal) = oneshot::channel();
    let http_task = tokio::spawn(async move {
        axum::serve(
            http_listener,
            payvia_platform_bff::http::routes::router(
                runtime_client,
                identity_client,
                tenant_client,
                realtime_client,
                catalog_client,
                entitlements_client,
            )
            .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = http_signal.await;
        })
        .await
    });
    let mut default_headers = header::HeaderMap::new();
    default_headers.insert(
        header::HOST,
        format!("localhost:{}", http_addr.port())
            .parse()
            .expect("local host header is valid"),
    );
    let source_id = Uuid::new_v4();
    let source_bytes = source_id.as_bytes();
    let test_source_ip = std::net::Ipv4Addr::new(
        127,
        source_bytes[0],
        source_bytes[1],
        source_bytes[2].max(1),
    );
    let client = Client::builder()
        .local_address(std::net::IpAddr::V4(test_source_ip))
        .default_headers(default_headers.clone())
        .build()
        .expect("HTTP acceptance client builds");
    let email = format!("acceptance_{}@example.test", Uuid::new_v4().simple());
    let password = format!("A-valid-password-{}!", Uuid::new_v4().simple());
    credential_validation::verify(credential_validation::IdentityCredentialValidationContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        email: &email,
        password: &password,
    })
    .await;

    let register_response = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_register")
        .json(&serde_json::json!({"email": email, "password": password}))
        .send()
        .await
        .expect("HTTP registration response arrives");
    assert_eq!(register_response.status(), StatusCode::CREATED);

    let registration: serde_json::Value = register_response
        .json()
        .await
        .expect("registration response contains its resource identifier");
    let registered_user_id = Uuid::parse_str(
        registration["user_id"]
            .as_str()
            .expect("registration user id is present"),
    )
    .expect("registration user id is valid");

    outbox_transaction_rollback::verify(&client, &migration_pool, http_addr, &origin).await;
    duplicate_registration::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &email,
        &password,
        registered_user_id,
    )
    .await;
    concurrent_registration::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        registered_user_id,
    )
    .await;

    let tenant_cookies = tenant_session_acceptance::verify(
        &client,
        http_addr,
        &origin,
        &email,
        &password,
        test_source_ip,
        &redis_manager,
        &captured_events,
        registered_user_id,
    )
    .await;
    let session_cookie = tenant_cookies.session_cookie.clone();
    let csrf_cookie = tenant_cookies.csrf_cookie.clone();
    let operator_cookies = operator_session_acceptance::verify(
        &client,
        http_addr,
        &origin,
        &operator_email,
        &operator_password,
        operator_id,
        &mut authorization_client,
        &migration_pool,
        &tenant_cookies,
    )
    .await;
    let operator_session_cookie = operator_cookies.session_cookie;
    let operator_csrf_cookie = operator_cookies.csrf_cookie;

    let tenant_registration = tenant_registration_acceptance::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
    )
    .await;
    let legal_identification_number = tenant_registration.legal_identification_number;
    let created_tenant_id = tenant_registration.tenant_id;
    let tenant_body = tenant_registration.tenant_body;

    tenant_platform_access::verify(
        &client,
        http_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
        &operator_session_cookie,
        &operator_csrf_cookie,
        created_tenant_id,
    )
    .await;
    let tenant_admin_status_body = tenant_status_outbox_acceptance::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &operator_session_cookie,
        &operator_csrf_cookie,
        created_tenant_id,
        &tenant_body,
    )
    .await;

    let nats_client = async_nats::ConnectOptions::with_user_and_password(
        std::env::var("NATS_USER").expect("development NATS user is configured"),
        std::env::var("NATS_PASSWORD").expect("development NATS password is configured"),
    )
    .connect(std::env::var("PLATFORM_NATS_URL").expect("development JetStream URL is configured"))
    .await
    .expect("real development NATS accepts authenticated connection");
    let jetstream = jetstream::new(nats_client);
    let failed_namespace = format!("e2e_failure_{}", Uuid::new_v4().simple());
    let failed_topology = EventTopology::from_namespace(Some(&failed_namespace))
        .expect("failure-path namespace is valid");
    failed_topology
        .ensure_stream(&jetstream)
        .await
        .expect("failure-path stream is created in real JetStream");
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _cleanup = EventStreamCleanup {
            jetstream: jetstream.clone(),
            namespace: failed_namespace.clone(),
            armed: true,
        };
        panic!("exercise E2E stream cleanup during test unwinding");
    }));
    assert!(failure.is_err(), "the cleanup failure path was exercised");
    assert!(
        jetstream
            .get_stream(&failed_topology.stream_name)
            .await
            .is_err_and(is_stream_not_found),
        "panic cleanup removes only the stream created for that failed execution"
    );
    let event_namespace = format!("e2e_test_{}", Uuid::new_v4().simple());
    let event_topology = EventTopology::from_namespace(Some(&event_namespace))
        .expect("test event namespace is valid");

    outbox_database_failures::verify(outbox_database_failures::OutboxDatabaseFailureContext {
        migration_pool: &migration_pool,
        runtime_pool: &runtime_pool,
        runtime_role: &runtime_role,
        jetstream: &jetstream,
        event_topology: &event_topology,
    })
    .await;
    tenant_outbox_recovery::verify(tenant_outbox_recovery::TenantOutboxRecoveryContext {
        migration_pool: &migration_pool,
        runtime_pool: &runtime_pool,
        jetstream: &jetstream,
        failed_topology: &failed_topology,
        failed_namespace: &failed_namespace,
        created_tenant_id,
    })
    .await;
    identity_outbox_recovery::verify(&migration_pool, &runtime_pool, &jetstream).await;

    let event_delivery =
        tenant_event_delivery::verify(tenant_event_delivery::TenantEventDeliveryContext {
            client: &client,
            migration_pool: &migration_pool,
            runtime_pool: &runtime_pool,
            jetstream: &jetstream,
            event_topology: &event_topology,
            event_namespace,
            http_addr,
            session_cookie: &session_cookie,
            csrf_cookie: &csrf_cookie,
            created_tenant_id,
            registered_user_id,
            legal_identification_number: &legal_identification_number,
        })
        .await;
    let stream = event_delivery.stream;
    let mut event_stream_cleanup = event_delivery.event_stream_cleanup;
    let mut isolated_stream_cleanup = event_delivery.isolated_stream_cleanup;

    api_key_dto_validation::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
        &operator_session_cookie,
        &operator_csrf_cookie,
        created_tenant_id,
    )
    .await;

    let (api_key_id, api_key_secret) = api_key_creation::verify(
        &client,
        &migration_pool,
        http_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
        created_tenant_id,
    )
    .await;

    let scoped_api_key_authorization = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_scope_allowed".to_owned(),
        })
        .await
        .expect("API key with the requested scope authorizes the operation")
        .into_inner();
    assert_eq!(scoped_api_key_authorization.key_id, api_key_id.to_string());
    assert_eq!(
        scoped_api_key_authorization.tenant_id,
        created_tenant_id.to_string()
    );
    phase3_catalog::configure(
        &client,
        http_addr,
        &origin,
        &operator_session_cookie,
        &operator_csrf_cookie,
        created_tenant_id,
    )
    .await;
    phase3_entitlements::verify(phase3_entitlements::EntitlementAcceptanceContext {
        client: &client,
        http_addr,
        grpc_addr,
        origin: &origin,
        session_cookie: &session_cookie,
        csrf_cookie: &csrf_cookie,
        tenant_id: created_tenant_id,
        api_key_id,
        api_key_secret,
        authorization_client: authorization_client.clone(),
    })
    .await;

    let denied_api_key_scope = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.capture".to_owned(),
            correlation_id: "corr_api_key_scope_denied".to_owned(),
        })
        .await
        .expect_err("API key cannot authorize an operation outside its scopes");
    assert_eq!(denied_api_key_scope.code(), tonic::Code::PermissionDenied);
    let wrong_environment_api_key = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "test".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_wrong_environment".to_owned(),
        })
        .await
        .expect_err("live API key cannot authorize test environment operations");
    assert_eq!(
        wrong_environment_api_key.code(),
        tonic::Code::Unauthenticated
    );
    let cross_tenant_api_key = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: Uuid::new_v4().to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_cross_tenant".to_owned(),
        })
        .await
        .expect_err("API key cannot authorize a different tenant");
    assert_eq!(cross_tenant_api_key.code(), tonic::Code::Unauthenticated);
    let api_key_last_used: bool = sqlx::query_scalar(
        "SELECT last_used_at IS NOT NULL FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
    )
    .bind(created_tenant_id)
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("successful scoped authorization updates last_used_at");
    assert!(api_key_last_used);
    let api_key_scope_audit_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE outcome='allowed'),count(*) FILTER (WHERE outcome='denied') FROM identity_access.audit_events WHERE action='api_key.authorization.checked' AND resource_id=$1 AND correlation_id IN ('corr_api_key_scope_allowed','corr_api_key_scope_denied')",
    )
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("API key scope authorization audit outcomes are queryable");
    assert_eq!(api_key_scope_audit_counts, (1, 1));

    let foreign_api_key_list = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{}/api-keys",
            Uuid::new_v4()
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant key access to an unassociated tenant is rejected");
    assert_eq!(foreign_api_key_list.status(), StatusCode::FORBIDDEN);

    let api_key_list = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("API key metadata is available to the authorized tenant");
    assert_eq!(api_key_list.status(), StatusCode::OK);
    let api_key_metadata: serde_json::Value = api_key_list
        .json()
        .await
        .expect("API key metadata is safe JSON");
    assert_eq!(api_key_metadata[0]["status"], "active");
    assert_eq!(api_key_metadata[0]["prefix"], api_key_created["prefix"]);
    assert!(api_key_metadata[0].get("secret_once").is_none());
    assert!(api_key_metadata[0].get("secret_digest").is_none());

    let api_key_revoke = client
        .delete(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys/{api_key_id}"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_api_key_revoke")
        .send()
        .await
        .expect("tenant owner can revoke its key through BFF and gRPC");
    assert_eq!(api_key_revoke.status(), StatusCode::NO_CONTENT);
    let csrf_rejected_api_key_revoke = client
        .delete(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys/{api_key_id}"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("missing CSRF token rejects API key revocation");
    assert_eq!(csrf_rejected_api_key_revoke.status(), StatusCode::FORBIDDEN);
    let revoked_key_status: String = sqlx::query_scalar(
        "SELECT status FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
    )
    .bind(created_tenant_id)
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("revocation persists in PostgreSQL");
    assert_eq!(revoked_key_status, "revoked");
    let revoked_api_key_authorization = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: created_tenant_id.to_string(),
            environment: "live".to_owned(),
            required_scope: "payment_intents.create".to_owned(),
            correlation_id: "corr_api_key_revoked_denied".to_owned(),
        })
        .await
        .expect_err("revoked API key cannot authorize future operations");
    assert_eq!(
        revoked_api_key_authorization.code(),
        tonic::Code::Unauthenticated
    );
    let api_key_audit: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE resource_id=$1 AND action='api_key.created' AND outcome='completed') AND EXISTS (SELECT 1 FROM identity_access.audit_events WHERE resource_id=$1 AND action='api_key.revoked' AND outcome='completed')",
    )
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("API key creation and revocation are audited");
    assert!(api_key_audit);

    tenant_role_matrix::verify(tenant_role_matrix::TenantRoleMatrixContext {
        client: &client,
        migration_pool: &migration_pool,
        authorization_client: &mut authorization_client,
        http_addr,
        session_cookie: &session_cookie,
        csrf_cookie: &csrf_cookie,
        origin: &origin,
        created_tenant_id,
        registered_user_id,
    })
    .await;

    let second_email = format!("second_tenant_{}@example.test", Uuid::new_v4().simple());
    let second_password = format!("Second-tenant-password-{}!", Uuid::new_v4().simple());
    let second_user_registration = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_second_tenant_register")
        .json(&serde_json::json!({"email": second_email, "password": second_password}))
        .send()
        .await
        .expect("second tenant user registers through the BFF");
    assert_eq!(second_user_registration.status(), StatusCode::CREATED);
    let second_login = client
        .post(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_second_tenant_login")
        .json(&serde_json::json!({"email": second_email, "password": second_password}))
        .send()
        .await
        .expect("second tenant user logs in through the BFF");
    assert_eq!(second_login.status(), StatusCode::OK);
    let second_cookies: Vec<String> = second_login
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let second_session_cookie = cookie_value(&second_cookies, "payvia_tenant_session");
    let second_csrf_cookie = cookie_value(&second_cookies, "payvia_tenant_csrf");
    let second_legal_identification_number = format!(
        "8{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let duplicate_slug_identification = format!(
        "8{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let conflicting_slug = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .header("x-csrf-token", &second_csrf_cookie)
        .header("x-correlation-id", "corr_duplicate_tenant_slug")
        .json(&serde_json::json!({
            "display_name": "Duplicate slug tenant",
            "slug": tenant_slug.clone(),
            "legal_identification_type": "cc",
            "legal_identification_number": duplicate_slug_identification
        }))
        .send()
        .await
        .expect("duplicate tenant slug returns a conflict");
    assert_eq!(conflicting_slug.status(), StatusCode::CONFLICT);
    let duplicate_slug_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants WHERE slug=$1")
            .bind(&tenant_slug)
            .fetch_one(&migration_pool)
            .await
            .expect("duplicate slug count is queryable");
    assert_eq!(duplicate_slug_count, 1, "tenant slug remains unique");
    let duplicate_slug_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
    )
    .bind(created_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant registration event count is queryable");
    assert_eq!(duplicate_slug_event_count, 1);

    tenant_registration_races::verify(tenant_registration_races::TenantRegistrationRaceContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        second_session_cookie: &second_session_cookie,
        second_csrf_cookie: &second_csrf_cookie,
    })
    .await;

    let second_tenant_create = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .header("x-csrf-token", &second_csrf_cookie)
        .header("x-correlation-id", "corr_second_tenant_onboarding")
        .json(&serde_json::json!({
            "display_name": "Second acceptance tenant",
            "slug": format!("second-acceptance-{}", Uuid::new_v4().simple()),
            "legal_identification_type": "cc",
            "legal_identification_number": second_legal_identification_number.clone()
        }))
        .send()
        .await
        .expect("second tenant onboarding reaches its owning module");
    assert_eq!(second_tenant_create.status(), StatusCode::CREATED);
    let second_tenant_body: serde_json::Value = second_tenant_create
        .json()
        .await
        .expect("second tenant response is safe JSON");
    let second_tenant_id = Uuid::parse_str(
        second_tenant_body["tenant_id"]
            .as_str()
            .expect("second tenant identifier is present"),
    )
    .expect("second tenant identifier is a UUID");
    let second_document: (String, String) = sqlx::query_as(
        "SELECT legal_identification_type, legal_identification_number FROM tenant_registry.tenants WHERE id=$1",
    )
    .bind(second_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant registry persists the CC document");
    assert_eq!(second_document.0, "cc");
    assert_eq!(second_document.1, second_legal_identification_number);
    platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
        &runtime_pool,
        &jetstream,
        &event_topology.subject_prefix,
    )
    .await
    .expect("second tenant registration publishes through the real JetStream relay");
    let second_tenant_event = stream
        .get_last_raw_message_by_subject(&registration_subject)
        .await
        .expect("JetStream durably stores second tenant registration");
    let second_tenant_envelope: serde_json::Value =
        serde_json::from_slice(&second_tenant_event.payload)
            .expect("second tenant event is a versioned envelope");
    assert_eq!(
        second_tenant_envelope["payload"]["tenant_id"],
        second_tenant_id.to_string()
    );
    assert!(platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
        &runtime_pool,
        &second_tenant_event.payload,
    )
    .await
    .expect("second membership is provisioned from JetStream event"));

    let cross_tenant_details = client
        .get(format!("http://{http_addr}/v1/tenants/{created_tenant_id}"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_cross_tenant_details")
        .send()
        .await
        .expect("second tenant user cannot read the first tenant");
    assert_eq!(cross_tenant_details.status(), StatusCode::FORBIDDEN);
    let cross_tenant_api_keys = client
        .get(format!("http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .send()
        .await
        .expect("second tenant user cannot enumerate first tenant API keys");
    assert_eq!(cross_tenant_api_keys.status(), StatusCode::FORBIDDEN);
    let mut isolation_transaction = runtime_pool
        .begin()
        .await
        .expect("runtime tenant-isolation transaction begins");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(second_tenant_id.to_string())
        .execute(&mut *isolation_transaction)
        .await
        .expect("runtime tenant context is set");
    let foreign_key_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys WHERE tenant_id=$1")
            .bind(created_tenant_id)
            .fetch_one(&mut *isolation_transaction)
            .await
            .expect("RLS filters API keys belonging to another tenant");
    assert_eq!(foreign_key_count, 0);
    isolation_transaction
        .commit()
        .await
        .expect("tenant-isolation transaction completes");

    let mut realtime_redis = redis_manager.clone();
    let first_journal_insert =
        platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
            &runtime_pool,
            &mut realtime_redis,
            &event_message.payload,
        )
        .await
        .expect("realtime consumer persists the event journal and publishes its Redis signal");
    assert!(first_journal_insert);
    let journal_payload: String =
        sqlx::query_scalar("SELECT payload::text FROM realtime.event_journal WHERE event_id=$1")
            .bind(registration_event_id)
            .fetch_one(&migration_pool)
            .await
            .expect("realtime journal contains the registered tenant event");
    assert!(!journal_payload.contains(&legal_identification_number));
    let duplicate_journal_insert =
        platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
            &runtime_pool,
            &mut realtime_redis,
            &event_message.payload,
        )
        .await
        .expect("redelivered JetStream event is safely deduplicated");
    assert!(!duplicate_journal_insert);
    let journal_event_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM realtime.event_journal WHERE event_id=$1")
            .bind(registration_event_id)
            .fetch_one(&migration_pool)
            .await
            .expect("journal stores the event id only once");
    assert_eq!(journal_event_count, 1);
    let replay_cursor = tenant_realtime_replay::verify(
        &client, http_addr, &session_cookie, &csrf_cookie, created_tenant_id,
    )
    .await;

    platform_realtime_replay::verify(
        &stream,
        &event_topology,
        &runtime_pool,
        &mut realtime_redis,
        &mut realtime_acceptance_client,
        &client,
        http_addr,
        &operator_session_cookie,
        &operator_csrf_cookie,
    )
    .await;

    operator_realtime_revocation::verify(
        &client,
        &mut realtime_acceptance_client,
        http_addr,
        &origin,
        &operator_session_cookie,
        &operator_csrf_cookie,
    )
    .await;

    let operator_logout_audited: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'identity.session.revoked' AND correlation_id = 'corr_operator_logout')",
    )
    .bind(operator_id)
    .fetch_one(&migration_pool)
    .await
    .expect("operator session revocation audit is queryable");
    assert!(
        operator_logout_audited,
        "operator session revocation is auditable"
    );
    let session_revocation_event: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.outbox_events WHERE event_type = 'identity.session.revoked' AND audience = 'platform_operator')",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("operator session revocation event is queryable");
    assert!(
        session_revocation_event,
        "operator session revocation reaches durable outbox"
    );
    let operator_replay = client
        .get(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .send()
        .await
        .expect("revoked operator session replay is rejected");
    assert_eq!(operator_replay.status(), StatusCode::UNAUTHORIZED);

    // The preceding logout deliberately revoked the first operator session.
    // Create a fresh session for the role/permission matrix that follows.
    let matrix_operator_login = client
        .post(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_operator_matrix_login")
        .json(&serde_json::json!({
            "email": operator_email.clone(),
            "password": operator_password.clone()
        }))
        .send()
        .await
        .expect("operator matrix login response arrives");
    assert_eq!(matrix_operator_login.status(), StatusCode::OK);
    let matrix_operator_cookies: Vec<String> = matrix_operator_login
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let matrix_operator_session_cookie =
        cookie_value(&matrix_operator_cookies, "payvia_platform_session");
    let matrix_operator_csrf_cookie =
        cookie_value(&matrix_operator_cookies, "payvia_platform_csrf");

    let rejected_csrf = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", "wrong-csrf-token")
        .send()
        .await
        .expect("CSRF rejection response arrives");
    assert_eq!(rejected_csrf.status(), StatusCode::FORBIDDEN);

    tenant_realtime_revocation::verify(
        &client,
        &mut realtime_acceptance_client,
        &migration_pool,
        http_addr,
        &origin,
        &session_cookie,
        &csrf_cookie,
        registered_user_id,
    )
    .await;

    login_rate_limits::verify(
        &client,
        &redis_manager,
        http_addr,
        &origin,
        &traceparent,
        &email,
        &ip_key,
    )
    .await;

    rate_limit_telemetry::verify(&captured_events);

    rate_limit_acceptance::verify(rate_limit_acceptance::RateLimitAcceptanceContext {
        http_addr,
        origin: &origin,
        default_headers,
        captured_events: &captured_events,
        password: &password,
        operator_password: &operator_password,
        session_cookie: &session_cookie,
        csrf_cookie: &csrf_cookie,
        operator_session_cookie: &operator_session_cookie,
        operator_csrf_cookie: &operator_csrf_cookie,
        legal_identification_number: &legal_identification_number,
        second_legal_identification_number: &second_legal_identification_number,
    })
    .await;

    operator_role_matrix::verify(operator_role_matrix::OperatorRoleMatrixContext {
        client: &client,
        migration_pool: &migration_pool,
        authorization_client: &mut authorization_client,
        operator_id,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        created_tenant_id,
        http_addr,
    })
    .await;

    tenant_status_lifecycle::verify(tenant_status_lifecycle::TenantStatusLifecycleContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
    })
    .await;

    let (tenant_before_invalid_transitions, current_status_version) =
        tenant_status_input_validation::verify(
            tenant_status_input_validation::TenantStatusInputContext {
                client: &client,
                migration_pool: &migration_pool,
                http_addr,
                origin: &origin,
                matrix_operator_session_cookie: &matrix_operator_session_cookie,
                matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
                created_tenant_id,
            },
        )
        .await;
    tenant_status_version_validation::verify(
        tenant_status_version_validation::TenantStatusVersionContext {
            client: &client,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            created_tenant_id,
            current_status_version,
        },
    )
    .await;
    tenant_dto_validation::verify(tenant_dto_validation::TenantDtoValidationContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        second_session_cookie: &second_session_cookie,
        second_csrf_cookie: &second_csrf_cookie,
        created_tenant_id,
        tenant_before_invalid_transitions,
    })
    .await;

    secondary_tenant_lifecycle::verify(
        secondary_tenant_lifecycle::SecondaryTenantLifecycleContext {
            client: &client,
            migration_pool: &migration_pool,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            operator_id,
            second_tenant_id,
        },
    )
    .await;

    session_security::verify(session_security::SessionSecurityContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        session_cookie: &session_cookie,
        csrf_cookie: &csrf_cookie,
        operator_session_cookie: &operator_session_cookie,
        operator_csrf_cookie: &operator_csrf_cookie,
        second_session_cookie: &second_session_cookie,
        second_csrf_cookie: &second_csrf_cookie,
        second_tenant_id,
        created_tenant_id,
        tenant_status_version: tenant_admin_status_body["status_version"].clone(),
    })
    .await;

    tenant_status_race::verify(tenant_status_race::TenantStatusRaceContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
        current_status_version,
    })
    .await;

    // A retained-journal gap must surface as a conflict at the HTTP edge,
    // not as a successful partial replay that silently skips an old event.
    let first_platform_event: Uuid =
        sqlx::query_scalar("SELECT event_id FROM realtime.event_journal WHERE platform_cursor=1")
            .fetch_one(&migration_pool)
            .await
            .expect("first platform journal event exists for gap injection");
    sqlx::query("DELETE FROM realtime.event_journal WHERE event_id=$1")
        .bind(first_platform_event)
        .execute(&migration_pool)
        .await
        .expect("first platform journal event is removed to simulate a retained gap");
    let platform_gap_replay = client
        .get(format!(
            "http://{http_addr}/v1/realtime/events?audience=platform&cursor=0"
        ))
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
            ),
        )
        .send()
        .await
        .expect("platform replay reports a retained journal gap");
    assert_eq!(platform_gap_replay.status(), StatusCode::CONFLICT);
    let platform_gap_body: serde_json::Value = platform_gap_replay
        .json()
        .await
        .expect("realtime gap returns a safe error envelope");
    assert_eq!(platform_gap_body["error"]["code"], "realtime_cursor_gap");

    let matrix_operator_logout = client
        .delete(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .send()
        .await
        .expect("operator matrix session can be revoked");
    assert_eq!(matrix_operator_logout.status(), StatusCode::NO_CONTENT);
    let revoked_matrix_session_response = client
        .get(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
            ),
        )
        .send()
        .await
        .expect("revoked matrix operator session receives an HTTP response");
    assert_eq!(
        revoked_matrix_session_response.status(),
        StatusCode::UNAUTHORIZED
    );
    let revoked_operator_tenant_list = client
        .get(format!("http://{http_addr}/v1/platform/tenants?limit=10&offset=0"))
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
            ),
        )
        .send()
        .await
        .expect("revoked operator session cannot access administrative tenant resources");
    assert_eq!(
        revoked_operator_tenant_list.status(),
        StatusCode::UNAUTHORIZED
    );

    outbox_leases::verify(&migration_pool, &runtime_pool, created_tenant_id).await;

    let mut isolated_stream_cleanup = isolated_stream_cleanup;
    isolated_stream_cleanup.cleanup().await;
    let mut event_stream_cleanup = event_stream_cleanup;
    event_stream_cleanup.cleanup().await;
    assert!(
        jetstream
            .get_stream(&event_topology.stream_name)
            .await
            .is_err_and(is_stream_not_found)
    );

    let _ = http_stop.send(());
    http_task
        .await
        .expect("HTTP task joins")
        .expect("BFF server stops cleanly");
    let _ = grpc_stop.send(());
    grpc_task
        .await
        .expect("gRPC task joins")
        .expect("runtime server stops cleanly");
    runtime_pool.close().await;
    migration_pool.close().await;
    database_cleanup.cleanup().await;
    admin_pool.close().await;
    provider.shutdown().expect("trace provider shuts down");
}
