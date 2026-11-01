#[path = "identity_access_acceptance/phase3_catalog.rs"]
mod phase3_catalog;
#[path = "identity_access_acceptance/phase3_entitlements.rs"]
mod phase3_entitlements;

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
    platform_identity_access::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("identity access migrations apply to real PostgreSQL");
    platform_tenant_registry::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("tenant registry migrations apply to real PostgreSQL");
    platform_realtime::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("realtime journal migration applies to real PostgreSQL");
    platform_catalog_pricing::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("catalog pricing migrations apply to real PostgreSQL");
    platform_entitlements_quotas::infrastructure::persistence::MIGRATOR
        .run(&migration_pool)
        .await
        .expect("entitlements and quotas migrations apply to real PostgreSQL");
    let tenant_role_permissions: Vec<(String, String)> = sqlx::query_as(
        "SELECT role, permission FROM identity_access.tenant_role_permissions ORDER BY role, permission",
    )
    .fetch_all(&migration_pool)
    .await
    .expect("tenant role permission matrix is queryable");
    assert_eq!(
        tenant_role_permissions,
        vec![
            ("developer".to_owned(), "api_keys.manage".to_owned()),
            ("developer".to_owned(), "tenant.read".to_owned()),
            ("developer".to_owned(), "usage.consume".to_owned()),
            ("developer".to_owned(), "usage.read".to_owned()),
            ("tenant_admin".to_owned(), "api_keys.manage".to_owned()),
            ("tenant_admin".to_owned(), "members.manage".to_owned()),
            ("tenant_admin".to_owned(), "tenant.read".to_owned()),
            (
                "tenant_admin".to_owned(),
                "tenant.settings.manage".to_owned()
            ),
            ("tenant_admin".to_owned(), "usage.consume".to_owned()),
            ("tenant_admin".to_owned(), "usage.read".to_owned()),
            ("tenant_owner".to_owned(), "api_keys.manage".to_owned()),
            ("tenant_owner".to_owned(), "members.manage".to_owned()),
            ("tenant_owner".to_owned(), "tenant.read".to_owned()),
            (
                "tenant_owner".to_owned(),
                "tenant.settings.manage".to_owned()
            ),
            ("tenant_owner".to_owned(), "usage.consume".to_owned()),
            ("tenant_owner".to_owned(), "usage.read".to_owned()),
            ("viewer".to_owned(), "tenant.read".to_owned()),
            ("viewer".to_owned(), "usage.read".to_owned()),
        ],
        "tenant role-to-permission grants must remain explicit and least-privilege"
    );
    let operator_role_permissions: Vec<(String, String)> = sqlx::query_as(
        "SELECT role, permission FROM identity_access.operator_role_permissions ORDER BY role, permission",
    )
    .fetch_all(&migration_pool)
    .await
    .expect("operator role permission matrix is queryable");
    assert_eq!(
        operator_role_permissions,
        vec![
            ("platform_owner".to_owned(), "catalog.manage".to_owned()),
            (
                "platform_owner".to_owned(),
                "operators.bootstrap".to_owned()
            ),
            ("platform_owner".to_owned(), "tenants.activate".to_owned()),
            ("platform_owner".to_owned(), "tenants.close".to_owned()),
            ("platform_owner".to_owned(), "tenants.read".to_owned()),
            ("platform_owner".to_owned(), "tenants.suspend".to_owned()),
            ("platform_owner".to_owned(), "tenants.verify".to_owned()),
            ("platform_security".to_owned(), "catalog.manage".to_owned()),
            (
                "platform_security".to_owned(),
                "tenants.activate".to_owned()
            ),
            ("platform_security".to_owned(), "tenants.close".to_owned()),
            ("platform_security".to_owned(), "tenants.read".to_owned()),
            ("platform_security".to_owned(), "tenants.suspend".to_owned()),
            ("platform_security".to_owned(), "tenants.verify".to_owned()),
            ("platform_support".to_owned(), "tenants.read".to_owned()),
        ],
        "operator role-to-permission grants must remain explicit and least-privilege"
    );

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

    let operator_email = format!("operator_{}@example.test", Uuid::new_v4().simple());
    let operator_password = format!("Operator-password-{}!", Uuid::new_v4().simple());
    let bootstrap_service = OperatorBootstrapService::new(runtime_pool.clone());
    let operator_id = bootstrap_service
        .bootstrap_first_operator(
            &operator_email,
            &operator_password,
            "corr_operator_bootstrap",
        )
        .await
        .expect("first operator bootstrap uses the real application service");
    assert!(
        matches!(
            bootstrap_service
                .bootstrap_first_operator(
                    "second_operator@example.test",
                    "Another-valid-password-123!",
                    "corr_operator_bootstrap_replay"
                )
                .await,
            Err(platform_identity_access::application::identity_service::IdentityError::OperatorBootstrapClosed)
        ),
        "bootstrap only succeeds once"
    );
    let operator_bootstrap_event: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.outbox_events WHERE aggregate_id = $1 AND event_type = 'identity.operator.bootstrap.completed')",
    )
    .bind(operator_id)
    .fetch_one(&migration_pool)
    .await
    .expect("operator bootstrap outbox event is queryable");
    assert!(
        operator_bootstrap_event,
        "operator bootstrap is durable for realtime"
    );

    let redis_client = redis::Client::open(redis_url.clone()).expect("Redis URL parses");
    let redis_manager = redis::aio::ConnectionManager::new(redis_client.clone())
        .await
        .expect("real development Redis accepts a connection");

    // A dedicated ACL user can connect but cannot execute the Lua rate-limit
    // script. Registration and login must fail closed before touching PostgreSQL.
    let restricted_redis_user = format!("phase2_limited_{}", Uuid::new_v4().simple());
    let restricted_redis_password = Uuid::new_v4().simple().to_string();
    let mut redis_acl_admin = redis_manager.clone();
    let _: String = redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&restricted_redis_user)
        .arg("on")
        .arg(format!(">{restricted_redis_password}"))
        .arg("~*")
        .arg("&*")
        .arg("+@connection")
        .arg("+SELECT")
        .query_async(&mut redis_acl_admin)
        .await
        .expect("isolated Redis user is created with connection-only permissions");
    let mut restricted_redis_url =
        url::Url::parse(&redis_url).expect("Redis URL parses for isolated ACL user");
    restricted_redis_url
        .set_username(&restricted_redis_user)
        .expect("restricted Redis username is valid");
    restricted_redis_url
        .set_password(Some(&restricted_redis_password))
        .expect("restricted Redis password is valid");
    let restricted_redis_client =
        redis::Client::open(restricted_redis_url.as_str()).expect("restricted Redis URL parses");
    let restricted_redis_manager = redis::aio::ConnectionManager::new(restricted_redis_client)
        .await
        .expect("restricted Redis user can establish a connection");
    let restricted_identity_service =
        platform_identity_access::application::identity_service::IdentityService::new(
            runtime_pool.clone(),
            restricted_redis_manager,
        );
    let users_before_redis_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.tenant_users")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant identity count before Redis failure is queryable");
    let redis_registration_failure = restricted_identity_service
        .register_tenant_user(
            &format!("redis_failure_{}@example.test", Uuid::new_v4().simple()),
            "Valid-password-123!",
            "127.0.0.1",
            "corr_identity_redis_registration_failure",
            None,
        )
        .await
        .expect_err("registration fails when Redis rate limiting is unavailable");
    assert!(matches!(
        redis_registration_failure,
        platform_identity_access::application::identity_service::IdentityError::Redis(_)
    ));
    let redis_login_failure = match restricted_identity_service
        .create_tenant_session(&operator_email, &operator_password, "127.0.0.1")
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("login must fail when Redis rate limiting is unavailable"),
    };
    assert!(matches!(
        redis_login_failure,
        platform_identity_access::application::identity_service::IdentityError::Redis(_)
    ));
    let users_after_redis_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.tenant_users")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant identity count after Redis failure is queryable");
    assert_eq!(
        users_after_redis_failure, users_before_redis_failure,
        "Redis rate-limit dependency failure must not create identities"
    );
    let _: i64 = redis::cmd("ACL")
        .arg("DELUSER")
        .arg(&restricted_redis_user)
        .query_async(&mut redis_acl_admin)
        .await
        .expect("isolated restricted Redis user is removed");

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
    let realtime_acceptance_client = realtime_client.clone();
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
    let identity_state_before_origin_rejection: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM identity_access.tenant_users),(SELECT count(*) FROM identity_access.sessions)",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity and session counts before Origin rejection are queryable");
    let invalid_origin_registration = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, "https://attacker.example")
        .json(&serde_json::json!({
            "email": email.clone(),
            "password": password.clone()
        }))
        .send()
        .await
        .expect("untrusted Origin registration receives an HTTP response");
    assert_eq!(invalid_origin_registration.status(), StatusCode::FORBIDDEN);
    let invalid_origin_tenant_login = client
        .post(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, "https://attacker.example")
        .json(&serde_json::json!({
            "email": email.clone(),
            "password": password.clone()
        }))
        .send()
        .await
        .expect("untrusted Origin tenant login receives an HTTP response");
    assert_eq!(invalid_origin_tenant_login.status(), StatusCode::FORBIDDEN);
    let identity_state_after_origin_rejection: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM identity_access.tenant_users),(SELECT count(*) FROM identity_access.sessions)",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity and session counts after Origin rejection are queryable");
    assert_eq!(
        identity_state_after_origin_rejection, identity_state_before_origin_rejection,
        "untrusted Origin must not create identities or sessions"
    );
    let identities_before_invalid_credentials: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.tenant_users")
            .fetch_one(&migration_pool)
            .await
            .expect("identity count before invalid credentials is queryable");
    let invalid_credential_types = [
        serde_json::json!({"email": 7, "password": "Valid-password-123!"}),
        serde_json::json!({"email": "invalid@example.test", "password": 7}),
        serde_json::json!({"email": "invalid@example.test"}),
    ];
    for (index, invalid_body) in invalid_credential_types.into_iter().enumerate() {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/register"))
            .header(header::ORIGIN, &origin)
            .json(&invalid_body)
            .send()
            .await
            .expect("invalid credentials DTO receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong credentials DTO type/shape case {index} must be rejected"
        );
    }
    let invalid_email = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .json(&serde_json::json!({
            "email": format!("{}@example.test", "a".repeat(65)),
            "password": "Valid-password-123!"
        }))
        .send()
        .await
        .expect("overlong email is rejected at the HTTP boundary");
    assert_eq!(invalid_email.status(), StatusCode::BAD_REQUEST);
    let invalid_passwords = vec!["Short-1!".to_owned(), format!("{}A1!", "a".repeat(126))];
    for invalid_password in invalid_passwords {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/register"))
            .header(header::ORIGIN, &origin)
            .json(&serde_json::json!({
                "email": format!("invalid_password_{}@example.test", Uuid::new_v4().simple()),
                "password": invalid_password
            }))
            .send()
            .await
            .expect("out-of-range password receives an HTTP response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let identities_after_invalid_credentials: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.tenant_users")
            .fetch_one(&migration_pool)
            .await
            .expect("identity count after invalid credentials is queryable");
    assert_eq!(
        identities_after_invalid_credentials, identities_before_invalid_credentials,
        "invalid credential values and types must not create identities"
    );

    let weak_password_email = format!("weak_password_{}@example.test", Uuid::new_v4().simple());
    let weak_password_response = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_weak_password")
        .json(&serde_json::json!({
            "email": weak_password_email,
            "password": "long-lowercase-password"
        }))
        .send()
        .await
        .expect("backend validates password complexity during account registration");
    assert_eq!(weak_password_response.status(), StatusCode::BAD_REQUEST);
    let weak_password_envelope: serde_json::Value = weak_password_response
        .json()
        .await
        .expect("invalid password returns a safe error envelope");
    assert_eq!(
        weak_password_envelope["error"]["correlation_id"],
        "corr_identity_weak_password"
    );
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

    // Force the Outbox insert to fail after the identity INSERT. The transaction
    // must roll back the identity and audit writes rather than leave partial state.
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION identity_access.reject_test_outbox_event() RETURNS trigger \
         LANGUAGE plpgsql AS $phase2$ BEGIN \
         IF NEW.correlation_id = 'corr_force_identity_outbox_failure' THEN \
           RAISE EXCEPTION 'forced acceptance failure'; \
         END IF; RETURN NEW; END $phase2$; \
         CREATE TRIGGER reject_test_outbox_event BEFORE INSERT ON identity_access.outbox_events \
         FOR EACH ROW EXECUTE FUNCTION identity_access.reject_test_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("temporary failure trigger is installed in the isolated acceptance database");
    let rollback_email = format!("rollback_{}@example.test", Uuid::new_v4().simple());
    let rollback_response = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_force_identity_outbox_failure")
        .json(&serde_json::json!({
            "email": rollback_email.clone(),
            "password": format!("Rollback-password-{}!", Uuid::new_v4().simple())
        }))
        .send()
        .await
        .expect("forced Outbox failure returns an HTTP response");
    assert_eq!(
        rollback_response.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let rollback_identity_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
    )
    .bind(rollback_email.to_ascii_lowercase())
    .fetch_one(&migration_pool)
    .await
    .expect("rollback identity count is queryable");
    assert_eq!(rollback_identity_count, 0, "identity insert must roll back");
    let rollback_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.audit_events WHERE correlation_id='corr_force_identity_outbox_failure'",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("rollback audit count is queryable");
    assert_eq!(rollback_audit_count, 0, "audit insert must not be partial");
    let rollback_outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.outbox_events WHERE correlation_id='corr_force_identity_outbox_failure'",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("rollback Outbox count is queryable");
    assert_eq!(
        rollback_outbox_count, 0,
        "Outbox insert must not be partial"
    );
    sqlx::raw_sql(
        "DROP TRIGGER reject_test_outbox_event ON identity_access.outbox_events; \
         DROP FUNCTION identity_access.reject_test_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("temporary failure trigger is removed");

    // Duplicate normalized email must be rejected at the HTTP boundary without
    // creating a second identity or a second durable registration event.
    let duplicate_registration = client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_duplicate_email")
        .json(&serde_json::json!({
            "email": format!(" {} ", email.to_ascii_uppercase()),
            "password": password.clone()
        }))
        .send()
        .await
        .expect("normalized duplicate email registration receives a classified response");
    assert_eq!(duplicate_registration.status(), StatusCode::CONFLICT);
    let duplicate_envelope: serde_json::Value = duplicate_registration
        .json()
        .await
        .expect("duplicate registration returns a safe error envelope");
    assert_eq!(duplicate_envelope["error"]["code"], "identity_conflict");
    let duplicate_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
    )
    .bind(email.to_ascii_lowercase())
    .fetch_one(&migration_pool)
    .await
    .expect("duplicate identity count is queryable");
    assert_eq!(duplicate_count, 1, "normalized email remains unique");
    let duplicate_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.outbox_events WHERE aggregate_id=$1 AND event_type='identity.tenant_user.registered'",
    )
    .bind(registered_user_id)
    .fetch_one(&migration_pool)
    .await
    .expect("registration event count is queryable");
    assert_eq!(
        duplicate_event_count, 1,
        "duplicate registration emits no event"
    );

    // Two simultaneous registrations for the same normalized email must have
    // exactly one winner, even when both requests pass the rate limiter.
    let concurrent_email = format!("concurrent_{}@example.test", Uuid::new_v4().simple());
    let concurrent_password = format!("Concurrent-password-{}!", Uuid::new_v4().simple());
    let concurrent_url = format!("http://{http_addr}/v1/identity/register");
    let first_request = client
        .post(&concurrent_url)
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_concurrent_first")
        .json(&serde_json::json!({
            "email": concurrent_email.clone(),
            "password": concurrent_password.clone()
        }))
        .send();
    let second_request = client
        .post(&concurrent_url)
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_concurrent_second")
        .json(&serde_json::json!({
            "email": concurrent_email.clone(),
            "password": concurrent_password.clone()
        }))
        .send();
    let (first_result, second_result) = tokio::join!(first_request, second_request);
    let first_status = first_result
        .expect("first concurrent registration response arrives")
        .status();
    let second_status = second_result
        .expect("second concurrent registration response arrives")
        .status();
    let mut concurrent_statuses = vec![first_status, second_status];
    concurrent_statuses.sort();
    assert_eq!(
        concurrent_statuses,
        vec![StatusCode::CREATED, StatusCode::CONFLICT],
        "one concurrent registration succeeds and the other conflicts"
    );
    let concurrent_identity_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.tenant_users WHERE email_normalized=$1",
    )
    .bind(concurrent_email.to_ascii_lowercase())
    .fetch_one(&migration_pool)
    .await
    .expect("concurrent identity count is queryable");
    assert_eq!(concurrent_identity_count, 1);

    let registration_event: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.outbox_events WHERE aggregate_id = $1 AND event_type = 'identity.tenant_user.registered')",
    )
    .bind(registered_user_id)
    .fetch_one(&migration_pool)
    .await
    .expect("registration outbox event is queryable");
    assert!(
        registration_event,
        "registration is written to durable outbox"
    );
    let registration_audit: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'tenant_user.registered' AND correlation_id = 'corr_identity_register')",
    )
    .bind(registered_user_id)
    .fetch_one(&migration_pool)
    .await
    .expect("registration audit event is queryable");
    assert!(registration_audit, "registration is auditable");

    let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
    let traceparent = format!("00-{trace_id}-00f067aa0ba902b7-01");
    let wrong_login = client
        .post(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_wrong_password")
        .header("traceparent", &traceparent)
        .json(&serde_json::json!({"email": email, "password": "incorrect-password"}))
        .send()
        .await
        .expect("invalid password gets a classified response");
    assert_eq!(wrong_login.status(), StatusCode::UNAUTHORIZED);
    let error_body: serde_json::Value = wrong_login.json().await.expect("safe error envelope");
    assert_eq!(error_body["error"]["code"], "authentication_failed");
    assert_eq!(
        error_body["error"]["correlation_id"],
        "corr_identity_wrong_password"
    );
    assert_eq!(error_body["error"]["trace_id"], trace_id);

    let account_digest = Sha256::digest(email.to_ascii_lowercase().as_bytes());
    let account_key = format!(
        "platform:identity:login:account:{}",
        URL_SAFE_NO_PAD.encode(account_digest)
    );
    let ip_digest = Sha256::digest(test_source_ip.to_string().as_bytes());
    let ip_key = format!(
        "platform:identity:login:ip:{}",
        URL_SAFE_NO_PAD.encode(ip_digest)
    );
    let mut redis_connection = redis_manager.clone();
    for key in [&account_key, &ip_key] {
        let ttl: i64 = redis::cmd("TTL")
            .arg(key)
            .query_async(&mut redis_connection)
            .await
            .expect("real Redis returns the rate-limit counter TTL");
        assert!(
            (1..=60).contains(&ttl),
            "rate-limit Redis counter must expire within its configured window"
        );
    }

    let events = captured_events.lock().expect("event capture lock").clone();
    let module_failure = events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("grpc.module.request.failed"))
                && event
                    .get("operation")
                    .is_some_and(|operation| operation.contains("create_tenant_session"))
        })
        .expect("causal identity failure event");
    let http_closure = events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("http.request.completed"))
                && event
                    .get("correlation_id")
                    .is_some_and(|value| value.contains("corr_identity_wrong_password"))
        })
        .expect("correlated HTTP closure event");
    for field in ["correlation_id", "trace_id"] {
        assert_eq!(module_failure.get(field), http_closure.get(field));
    }
    assert!(
        http_closure
            .get("error_code")
            .is_some_and(|code| code.contains("authentication_failed"))
    );
    assert!(
        http_closure
            .get("error_category")
            .is_some_and(|category| category.contains("authentication"))
    );

    let login = client
        .post(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_identity_login")
        .json(&serde_json::json!({"email": email, "password": password}))
        .send()
        .await
        .expect("valid login response arrives");
    assert_eq!(login.status(), StatusCode::OK);
    let cookies: Vec<String> = login
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let session_cookie = cookie_value(&cookies, "payvia_tenant_session");
    let csrf_cookie = cookie_value(&cookies, "payvia_tenant_csrf");
    assert!(
        cookies
            .iter()
            .any(|value| value.starts_with("payvia_tenant_session=")
                && value.contains("HttpOnly")
                && value.contains("Secure"))
    );
    assert!(
        cookies
            .iter()
            .any(|value| value.starts_with("payvia_tenant_csrf=") && value.contains("Secure"))
    );

    let tenant_session = client
        .get(format!("http://{http_addr}/v1/identity/session"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant session can be recovered after app reload");
    assert_eq!(tenant_session.status(), StatusCode::OK);
    let tenant_session_body: serde_json::Value = tenant_session
        .json()
        .await
        .expect("tenant session response contains authorized identity metadata");
    assert_eq!(
        tenant_session_body["user_id"],
        registered_user_id.to_string()
    );

    let tenant_session_used_for_operator = client
        .get(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(
            header::COOKIE,
            format!("payvia_platform_session={session_cookie}; payvia_platform_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant session cannot access operator identity");
    assert_eq!(
        tenant_session_used_for_operator.status(),
        StatusCode::UNAUTHORIZED
    );

    let invalid_origin_operator_login = client
        .post(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, "https://attacker.example")
        .json(&serde_json::json!({
            "email": operator_email.clone(),
            "password": operator_password.clone()
        }))
        .send()
        .await
        .expect("untrusted Origin operator login receives an HTTP response");
    assert_eq!(
        invalid_origin_operator_login.status(),
        StatusCode::FORBIDDEN
    );

    let operator_login = client
        .post(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_operator_login")
        .json(&serde_json::json!({
            "email": operator_email.clone(),
            "password": operator_password.clone()
        }))
        .send()
        .await
        .expect("operator login response arrives");
    assert_eq!(operator_login.status(), StatusCode::OK);
    let operator_cookies: Vec<String> = operator_login
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let operator_session_cookie = cookie_value(&operator_cookies, "payvia_platform_session");
    let operator_csrf_cookie = cookie_value(&operator_cookies, "payvia_platform_csrf");

    let authorization = authorization_client
        .authorize_operator_operation(AuthorizeOperatorOperationRequest {
            session_token: operator_session_cookie.clone(),
            csrf_token: operator_csrf_cookie.clone(),
            permission: "tenants.activate".to_owned(),
            correlation_id: "corr_operator_authorization".to_owned(),
        })
        .await
        .expect("operator permission is checked in the identity module")
        .into_inner();
    assert_eq!(authorization.operator_id, operator_id.to_string());
    let denied_authorization = authorization_client
        .authorize_operator_operation(AuthorizeOperatorOperationRequest {
            session_token: operator_session_cookie.clone(),
            csrf_token: operator_csrf_cookie.clone(),
            permission: "billing.refunds.issue".to_owned(),
            correlation_id: "corr_operator_authorization_denied".to_owned(),
        })
        .await
        .expect_err("unassigned permission is denied");
    assert_eq!(denied_authorization.code(), tonic::Code::PermissionDenied);
    let authorization_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.audit_events WHERE actor_id=$1 AND action='operator.authorization.checked' AND correlation_id IN ('corr_operator_authorization','corr_operator_authorization_denied')",
    )
    .bind(operator_id)
    .fetch_one(&migration_pool)
    .await
    .expect("authorization decisions are auditable");
    assert_eq!(authorization_audit_count, 2);
    let operator_session = client
        .get(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .send()
        .await
        .expect("operator session validates through the BFF");
    assert_eq!(operator_session.status(), StatusCode::OK);
    let operator_session_used_for_tenant = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_tenant_session={operator_session_cookie}; payvia_tenant_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", &operator_csrf_cookie)
        .send()
        .await
        .expect("operator session cannot access tenant identity");
    assert_eq!(
        operator_session_used_for_tenant.status(),
        StatusCode::UNAUTHORIZED
    );
    let legal_identification_number = format!(
        "9{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let tenant_slug = format!("acceptance-{}", Uuid::new_v4().simple());
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION tenant_registry.reject_test_outbox_event() RETURNS trigger \
         LANGUAGE plpgsql AS $tenant_register$ BEGIN \
         IF NEW.correlation_id = 'corr_tenant_registration_outbox_failure' THEN \
             RAISE EXCEPTION 'injected tenant Outbox failure'; \
         END IF; RETURN NEW; END $tenant_register$; \
         CREATE TRIGGER reject_test_outbox_event BEFORE INSERT ON tenant_registry.outbox_events \
         FOR EACH ROW EXECUTE FUNCTION tenant_registry.reject_test_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("tenant Outbox failure trigger is installed");
    let tenants_before_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count before Outbox failure is queryable");
    let tenant_outbox_failure = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_tenant_registration_outbox_failure")
        .json(&serde_json::json!({
            "display_name": "Tenant rollback acceptance",
            "slug": format!("rollback-{}", Uuid::new_v4().simple()),
            "legal_identification_type": "nit",
            "legal_identification_number": format!("7{:018}", Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000)
        }))
        .send()
        .await
        .expect("tenant registration Outbox failure receives an HTTP response");
    assert_eq!(
        tenant_outbox_failure.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let tenants_after_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count after Outbox failure is queryable");
    assert_eq!(
        tenants_after_outbox_failure, tenants_before_outbox_failure,
        "tenant registration must roll back when its Outbox write fails"
    );
    let failed_tenant_outbox_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE correlation_id='corr_tenant_registration_outbox_failure'",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("failed tenant Outbox write leaves no row");
    assert_eq!(failed_tenant_outbox_rows, 0);
    sqlx::raw_sql(
        "DROP TRIGGER reject_test_outbox_event ON tenant_registry.outbox_events;          DROP FUNCTION tenant_registry.reject_test_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("tenant registration failure trigger is removed");
    let tenant_create = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_tenant_onboarding")
        .json(&serde_json::json!({
            "display_name": "Acceptance tenant",
            "slug": tenant_slug.clone(),
            "legal_identification_type": "nit",
            "legal_identification_number": legal_identification_number.clone()
        }))
        .send()
        .await
        .expect("tenant onboarding reaches tenant registry through BFF and gRPC");
    assert_eq!(tenant_create.status(), StatusCode::CREATED);
    let tenant_body: serde_json::Value = tenant_create
        .json()
        .await
        .expect("tenant onboarding returns a safe tenant representation");
    assert!(tenant_body.get("legal_identification_number").is_none());
    assert!(tenant_body.get("legal_identification_type").is_none());
    let created_tenant_id = Uuid::parse_str(
        tenant_body["tenant_id"]
            .as_str()
            .expect("created tenant identifier is present"),
    )
    .expect("created tenant identifier is a UUID");
    assert_eq!(tenant_body["status"], "pending_verification");
    let stored_identification: String = sqlx::query_scalar(
        "SELECT legal_identification_number FROM tenant_registry.tenants WHERE id=$1",
    )
    .bind(created_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant registry persists the submitted legal identification");
    assert_eq!(stored_identification, legal_identification_number);
    let tenant_event_payload: String = sqlx::query_scalar(
        "SELECT payload::text FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
    )
    .bind(created_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant registration outbox event exists");
    assert!(!tenant_event_payload.contains(&legal_identification_number));
    let owner_membership_event: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered' AND correlation_id='corr_tenant_onboarding')",
    )
    .bind(created_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant onboarding is durably queued for owner membership provisioning");
    assert!(owner_membership_event);
    let conflicting_document = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_duplicate_tenant_document")
        .json(&serde_json::json!({
            "display_name": "Another acceptance tenant",
            "slug": format!("another-acceptance-{}", Uuid::new_v4().simple()),
            "legal_identification_type": "nit",
            "legal_identification_number": legal_identification_number.clone()
        }))
        .send()
        .await
        .expect("duplicate legal identification returns a conflict");
    assert_eq!(conflicting_document.status(), StatusCode::CONFLICT);
    let conflict_envelope: serde_json::Value = conflicting_document
        .json()
        .await
        .expect("duplicate legal identification returns a safe error envelope");
    assert_eq!(
        conflict_envelope["error"]["correlation_id"],
        "corr_duplicate_tenant_document"
    );
    assert!(
        !conflict_envelope
            .to_string()
            .contains(&legal_identification_number)
    );
    let tenant_session_platform_list = client
        .get(format!(
            "http://{http_addr}/v1/platform/tenants?limit=10&offset=0"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant session cannot enumerate platform tenants");
    assert_eq!(
        tenant_session_platform_list.status(),
        StatusCode::UNAUTHORIZED
    );
    let tenant_session_platform_mutation = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Tenant cannot administer platform state",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("tenant session cannot mutate platform tenant state");
    assert_eq!(
        tenant_session_platform_mutation.status(),
        StatusCode::FORBIDDEN
    );
    let tenant_list = client
        .get(format!("http://{http_addr}/v1/platform/tenants?query=Acceptance&limit=10&offset=0"))
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_tenant_admin_list")
        .send()
        .await
        .expect("operator can list tenants through BFF and gRPC");
    assert_eq!(tenant_list.status(), StatusCode::OK);
    let tenant_list_body: serde_json::Value =
        tenant_list.json().await.expect("tenant list is JSON");
    assert_eq!(tenant_list_body["total"], 1);
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION tenant_registry.reject_test_status_outbox_event() RETURNS trigger \
         LANGUAGE plpgsql AS $tenant_status$ BEGIN \
         IF NEW.correlation_id = 'corr_tenant_status_outbox_failure' THEN \
             RAISE EXCEPTION 'injected tenant status Outbox failure'; \
         END IF; RETURN NEW; END $tenant_status$; \
         CREATE TRIGGER reject_test_status_outbox_event BEFORE INSERT ON tenant_registry.outbox_events \
         FOR EACH ROW EXECUTE FUNCTION tenant_registry.reject_test_status_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("tenant status Outbox failure trigger is installed");
    let tenant_before_status_outbox_failure: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("tenant state before status Outbox failure is queryable");
    let status_outbox_failure = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &operator_csrf_cookie)
        .header("x-correlation-id", "corr_tenant_status_outbox_failure")
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Injected Outbox failure must roll back",
            "expected_version": tenant_before_status_outbox_failure.1
        }))
        .send()
        .await
        .expect("tenant status Outbox failure receives an HTTP response");
    assert_eq!(
        status_outbox_failure.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let tenant_after_status_outbox_failure: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("tenant state after status Outbox failure is queryable");
    assert_eq!(
        tenant_after_status_outbox_failure, tenant_before_status_outbox_failure,
        "status and version must roll back when the Outbox write fails"
    );
    let failed_status_history_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.status_history WHERE tenant_id=$1 AND version=$2",
    )
    .bind(created_tenant_id)
    .bind(tenant_before_status_outbox_failure.1 + 1)
    .fetch_one(&migration_pool)
    .await
    .expect("failed status Outbox write leaves no history row");
    assert_eq!(failed_status_history_rows, 0);
    sqlx::raw_sql(
        "DROP TRIGGER reject_test_status_outbox_event ON tenant_registry.outbox_events;          DROP FUNCTION tenant_registry.reject_test_status_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("tenant status failure trigger is removed");
    let tenant_admin_status = client
        .post(format!("http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &operator_csrf_cookie)
        .header("x-correlation-id", "corr_tenant_admin_activate")
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Acceptance verification",
            "expected_version": tenant_body["status_version"]
        }))
        .send()
        .await
        .expect("authorized operator can activate tenant");
    assert_eq!(tenant_admin_status.status(), StatusCode::OK);
    let tenant_admin_status_body: serde_json::Value = tenant_admin_status
        .json()
        .await
        .expect("status change returns the tenant state");
    assert_eq!(tenant_admin_status_body["status"], "active");
    assert_eq!(tenant_admin_status_body["status_version"], 2);
    let tenant_status_outbox: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.status.changed' AND correlation_id='corr_tenant_admin_activate')",
    )
    .bind(created_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant status mutation has a durable outbox event");
    assert!(tenant_status_outbox);
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

    // Simulate a transient PostgreSQL write failure in the Outbox relay.
    // The failed claim must not consume or mutate events; restoring privileges
    // lets the same pending events be processed by the next relay attempt.
    let pending_before_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM tenant_registry.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("pending Outbox state before PostgreSQL failure is queryable");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE ON tenant_registry.outbox_events FROM \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("isolated runtime role loses Outbox update privilege for failure injection");
    let postgres_relay_failure =
        platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
            &runtime_pool,
            &jetstream,
            &event_topology.subject_prefix,
        )
        .await;
    assert!(
        postgres_relay_failure.is_err(),
        "PostgreSQL permission failure is surfaced to the relay worker"
    );
    let pending_after_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM tenant_registry.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("pending Outbox state after PostgreSQL failure is queryable");
    assert_eq!(
        pending_after_database_failure, pending_before_database_failure,
        "failed PostgreSQL claim must not partially lease or consume events"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT UPDATE ON tenant_registry.outbox_events TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("isolated runtime role regains Outbox update privilege");

    let pending_identity_before_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM identity_access.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity Outbox state before PostgreSQL failure is queryable");
    assert!(pending_identity_before_database_failure.0 > 0);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE ON identity_access.outbox_events FROM \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("identity Outbox update privilege is revoked for failure injection");
    let identity_postgres_relay_failure =
        platform_identity_access::infrastructure::outbox_relay::relay_batch(
            &runtime_pool,
            &jetstream,
            &event_topology.subject_prefix,
        )
        .await;
    assert!(
        identity_postgres_relay_failure.is_err(),
        "identity relay surfaces a PostgreSQL claim failure"
    );
    let pending_identity_after_database_failure: (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(attempts),0) FROM identity_access.outbox_events WHERE published_at IS NULL",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("identity Outbox state after PostgreSQL failure is queryable");
    assert_eq!(
        pending_identity_after_database_failure, pending_identity_before_database_failure,
        "identity Outbox PostgreSQL failure must not partially claim or consume events"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT UPDATE ON identity_access.outbox_events TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("identity Outbox update privilege is restored");

    // Exercise a deterministic JetStream failure without racing the
    // application's background relay: the synthetic Outbox row is leased
    // while the missing-stream publish is attempted.
    let failed_event_id = Uuid::new_v4();
    let failed_aggregate_id = Uuid::new_v4();
    let failed_lease_token = Uuid::new_v4();
    let failed_correlation_id = "corr_jetstream_publish_retry";
    let failed_payload = serde_json::json!({"tenant_id": created_tenant_id});
    sqlx::query(
        "INSERT INTO tenant_registry.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id,lease_token,lease_until,attempts) VALUES ($1,$2,1,'tenant.registered',1,'tenant_and_platform',$3,$4,$5,now()+interval '30 seconds',1)",
    )
    .bind(failed_event_id)
    .bind(failed_aggregate_id)
    .bind(&failed_payload)
    .bind(failed_correlation_id)
    .bind(failed_lease_token)
    .execute(&migration_pool)
    .await
    .expect("synthetic Outbox event is leased for deterministic failure injection");
    let failed_event = platform_tenant_registry::infrastructure::persistence::PendingOutboxEvent {
        event_id: failed_event_id,
        aggregate_id: failed_aggregate_id,
        aggregate_version: 1,
        event_type: "tenant.registered".to_owned(),
        schema_version: 1,
        audience: "tenant_and_platform".to_owned(),
        payload: failed_payload.clone(),
        traceparent: None,
        correlation_id: failed_correlation_id.to_owned(),
        lease_token: failed_lease_token,
    };
    let jetstream_error_code =
        platform_tenant_registry::infrastructure::outbox_relay::publish_event(
            &jetstream,
            &failed_event,
            &failed_topology.subject_prefix,
        )
        .await
        .expect_err("publishing without a matching stream fails retryably");
    assert!(matches!(
        jetstream_error_code,
        "jetstream_publish_unavailable" | "jetstream_ack_unavailable"
    ));
    assert!(
        platform_tenant_registry::infrastructure::persistence::reschedule_outbox_event(
            &runtime_pool,
            failed_event_id,
            failed_lease_token,
            jetstream_error_code,
        )
        .await
        .expect("failed JetStream event is rescheduled"),
        "the failed event lease is released for retry"
    );
    let deferred_event_state: (Option<String>, bool) = sqlx::query_as(
        "SELECT last_error_code,published_at IS NULL FROM tenant_registry.outbox_events WHERE event_id=$1",
    )
    .bind(failed_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("deferred JetStream event state is queryable");
    assert_eq!(
        deferred_event_state.0.as_deref(),
        Some(jetstream_error_code)
    );
    assert!(deferred_event_state.1);

    let failed_stream = failed_topology
        .ensure_stream(&jetstream)
        .await
        .expect("JetStream stream is restored for retry");
    let mut failed_stream_cleanup = EventStreamCleanup {
        jetstream: jetstream.clone(),
        namespace: failed_namespace,
        armed: true,
    };
    let retry_lease_token = Uuid::new_v4();
    sqlx::query(
        "UPDATE tenant_registry.outbox_events SET available_at=now(),lease_token=$2,lease_until=now()+interval '30 seconds' WHERE event_id=$1 AND published_at IS NULL",
    )
    .bind(failed_event_id)
    .bind(retry_lease_token)
    .execute(&migration_pool)
    .await
    .expect("deferred event is leased again after JetStream recovery");
    let retry_event = platform_tenant_registry::infrastructure::persistence::PendingOutboxEvent {
        lease_token: retry_lease_token,
        ..failed_event
    };
    platform_tenant_registry::infrastructure::outbox_relay::publish_event(
        &jetstream,
        &retry_event,
        &failed_topology.subject_prefix,
    )
    .await
    .expect("JetStream retry succeeds after stream recovery");
    assert!(
        platform_tenant_registry::infrastructure::persistence::mark_outbox_published(
            &runtime_pool,
            failed_event_id,
            retry_lease_token,
        )
        .await
        .expect("successful retry is acknowledged in PostgreSQL"),
        "successful JetStream retry marks the event published"
    );
    let retried_jetstream_state: (bool, Option<String>) = sqlx::query_as(
        "SELECT published_at IS NOT NULL,last_error_code FROM tenant_registry.outbox_events WHERE event_id=$1",
    )
    .bind(failed_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("successful JetStream retry state is queryable");
    assert!(retried_jetstream_state.0);
    assert!(retried_jetstream_state.1.is_none());
    drop(failed_stream);
    failed_stream_cleanup.cleanup().await;

    // Exercise the identity_access Outbox worker against a real JetStream
    // publish failure and recovery, independently of tenant_registry.
    sqlx::query(
        "UPDATE identity_access.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL",
    )
    .execute(&migration_pool)
    .await
    .expect("existing identity Outbox events are deferred for isolated retry acceptance");
    let identity_failure_namespace = format!("e2e_identity_{}", Uuid::new_v4().simple());
    let identity_failure_topology =
        EventTopology::from_namespace(Some(&identity_failure_namespace))
            .expect("identity failure namespace is valid");
    let identity_failed_event_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity_access.outbox_events          (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id)          VALUES ($1,$2,1,'identity.session.revoked',1,'platform_operator',$3,'corr_identity_jetstream_retry')",
    )
    .bind(identity_failed_event_id)
    .bind(Uuid::new_v4())
    .bind(serde_json::json!({"session_id": Uuid::new_v4()}))
    .execute(&migration_pool)
    .await
    .expect("identity Outbox retry event is inserted");
    let failed_identity_delivery =
        platform_identity_access::infrastructure::outbox_relay::relay_batch(
            &runtime_pool,
            &jetstream,
            &identity_failure_topology.subject_prefix,
        )
        .await
        .expect("identity relay classifies and reschedules JetStream publish failure");
    assert_eq!(failed_identity_delivery, 0);
    let identity_deferred_state: (Option<String>, bool, bool) = sqlx::query_as(
        "SELECT last_error_code,published_at IS NULL,lease_token IS NULL FROM identity_access.outbox_events WHERE event_id=$1",
    )
    .bind(identity_failed_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("identity deferred Outbox state is queryable");
    assert_eq!(
        identity_deferred_state.0.as_deref(),
        Some("jetstream_ack_unavailable")
    );
    assert!(identity_deferred_state.1);
    assert!(identity_deferred_state.2);
    identity_failure_topology
        .ensure_stream(&jetstream)
        .await
        .expect("identity JetStream stream is restored for retry");
    sqlx::query("UPDATE identity_access.outbox_events SET available_at=now() WHERE event_id=$1")
        .bind(identity_failed_event_id)
        .execute(&migration_pool)
        .await
        .expect("identity failed event becomes eligible after dependency recovery");
    let identity_retried_delivery =
        platform_identity_access::infrastructure::outbox_relay::relay_batch(
            &runtime_pool,
            &jetstream,
            &identity_failure_topology.subject_prefix,
        )
        .await
        .expect("identity relay retries the durable event after JetStream recovery");
    assert_eq!(identity_retried_delivery, 1);
    let identity_retried_state: (bool, Option<String>) = sqlx::query_as(
        "SELECT published_at IS NOT NULL,last_error_code FROM identity_access.outbox_events WHERE event_id=$1",
    )
    .bind(identity_failed_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("identity successful retry state is queryable");
    assert!(identity_retried_state.0);
    assert!(identity_retried_state.1.is_none());
    delete_e2e_stream(&jetstream, &identity_failure_namespace)
        .await
        .expect("identity failure stream cleanup removes only its isolated stream");

    let stream = event_topology
        .ensure_stream(&jetstream)
        .await
        .expect("isolated real JetStream stream is available");
    let event_stream_cleanup = EventStreamCleanup {
        jetstream: jetstream.clone(),
        namespace: event_namespace,
        armed: true,
    };
    let isolated_namespace = format!("e2e_isolation_{}", Uuid::new_v4().simple());
    let isolated_topology = EventTopology::from_namespace(Some(&isolated_namespace))
        .expect("second test namespace is valid");
    isolated_topology
        .ensure_stream(&jetstream)
        .await
        .expect("second isolated JetStream stream is available");
    let isolated_stream_cleanup = EventStreamCleanup {
        jetstream: jetstream.clone(),
        namespace: isolated_namespace,
        armed: true,
    };
    let registration_subject = format!("{}.tenant.registered", event_topology.subject_prefix);
    let registration_event_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
    )
    .bind(created_tenant_id)
    .fetch_one(&migration_pool)
    .await
    .expect("onboarding event identifier is durable");
    let published_count = platform_tenant_registry::infrastructure::outbox_relay::relay_batch(
        &runtime_pool,
        &jetstream,
        &event_topology.subject_prefix,
    )
    .await
    .expect("tenant outbox relay publishes through the real JetStream client");
    assert!(
        published_count > 0,
        "outbox events are acknowledged durably after dependency recovery"
    );
    let retried_event_state: (bool, Option<String>) = sqlx::query_as(
        "SELECT published_at IS NOT NULL,last_error_code FROM tenant_registry.outbox_events WHERE event_id=$1",
    )
    .bind(registration_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("retried Outbox event state is queryable");
    assert!(retried_event_state.0);
    assert!(retried_event_state.1.is_none());
    let isolated_stream = jetstream
        .get_stream(&isolated_topology.stream_name)
        .await
        .expect("second isolated stream remains available");
    let isolated_subject = format!("{}.tenant.registered", isolated_topology.subject_prefix);
    let isolated_read = isolated_stream
        .get_last_raw_message_by_subject(&isolated_subject)
        .await;
    assert!(
        isolated_read.is_err_and(|error| {
            error.kind() == async_nats::jetstream::stream::LastRawMessageErrorKind::NoMessageFound
        }),
        "an event published to one E2E namespace is absent from another"
    );
    let outbox_published: bool = sqlx::query_scalar(
        "SELECT published_at IS NOT NULL FROM tenant_registry.outbox_events WHERE event_id=$1",
    )
    .bind(registration_event_id)
    .fetch_one(&migration_pool)
    .await
    .expect("outbox publication state can be verified");
    assert!(
        outbox_published,
        "Outbox is marked only after JetStream ack"
    );
    let event_message = stream
        .get_last_raw_message_by_subject(&registration_subject)
        .await
        .expect("JetStream durably stores tenant registration event");
    let event_envelope: serde_json::Value = serde_json::from_slice(&event_message.payload)
        .expect("published event uses the versioned JSON envelope");
    assert_eq!(
        event_envelope["event_id"],
        registration_event_id.to_string()
    );
    assert!(
        !String::from_utf8_lossy(&event_message.payload).contains(&legal_identification_number)
    );
    let membership_created = platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
        &runtime_pool,
        &event_message.payload,
    )
    .await
    .expect("real consumer provisions owner membership from the durable event");
    assert!(membership_created);
    let owner_membership: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.tenant_memberships WHERE tenant_id=$1 AND user_id=$2 AND role='tenant_owner' AND status='active')",
    )
    .bind(created_tenant_id)
    .bind(registered_user_id)
    .fetch_one(&migration_pool)
    .await
    .expect("owner membership is persisted in its owning module");
    assert!(owner_membership);
    let duplicate_membership = platform_identity_access::infrastructure::tenant_membership_consumer::consume_tenant_registered(
        &runtime_pool,
        &event_message.payload,
    )
    .await
    .expect("redelivery is handled idempotently");
    assert!(
        !duplicate_membership,
        "redelivered event does not duplicate membership"
    );
    let memberships_response = client
        .get(format!("http://{http_addr}/v1/identity/memberships"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_memberships_read")
        .send()
        .await
        .expect("tenant membership query reaches identity module through BFF");
    assert_eq!(memberships_response.status(), StatusCode::OK);
    let memberships_body: serde_json::Value = memberships_response
        .json()
        .await
        .expect("membership response is safe JSON");
    assert_eq!(
        memberships_body["memberships"][0]["tenant_id"],
        created_tenant_id.to_string()
    );
    assert_eq!(memberships_body["memberships"][0]["role"], "tenant_owner");

    let operator_session_api_key_list = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .send()
        .await
        .expect("operator session cannot use tenant API key endpoints");
    assert_eq!(
        operator_session_api_key_list.status(),
        StatusCode::UNAUTHORIZED
    );
    let operator_session_api_key_create = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .json(&serde_json::json!({
            "name": "Wrong actor",
            "environment": "test",
            "scopes": ["payment_intents.create", "usage.consume"]
        }))
        .send()
        .await
        .expect("operator session cannot create tenant API keys");
    assert_eq!(
        operator_session_api_key_create.status(),
        StatusCode::UNAUTHORIZED
    );

    let api_keys_before_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count before invalid DTO checks is queryable");
    let invalid_api_key_values = [
        serde_json::json!({"name": "x", "environment": "test", "scopes": ["read"]}),
        serde_json::json!({"name": "x".repeat(81), "environment": "test", "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": "production", "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": []}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": ["Read"]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": ["a".repeat(65)]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": vec!["read"; 17]}),
    ];
    for (index, invalid_body) in invalid_api_key_values.into_iter().enumerate() {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("semantically invalid API key DTO receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "invalid API key DTO case {index} must fail validation"
        );
    }
    let invalid_api_key_types = [
        serde_json::json!({"name": 7, "environment": "test", "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": 7, "scopes": ["read"]}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": "read"}),
        serde_json::json!({"name": "Valid key", "environment": "test", "scopes": [7]}),
    ];
    for (index, invalid_body) in invalid_api_key_types.into_iter().enumerate() {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("incorrect API key DTO field type receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong API key DTO type case {index} must be rejected by the HTTP extractor"
        );
    }
    let api_keys_after_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count after invalid DTO checks is queryable");
    assert_eq!(
        api_keys_after_invalid_dtos, api_keys_before_invalid_dtos,
        "invalid API key values and types must not persist API keys"
    );

    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION identity_access.reject_test_api_key_outbox_event() RETURNS trigger \
         LANGUAGE plpgsql AS $api_key$ BEGIN \
         IF NEW.correlation_id = 'corr_api_key_outbox_failure' THEN \
             RAISE EXCEPTION 'injected API key Outbox failure'; \
         END IF; RETURN NEW; END $api_key$; \
         CREATE TRIGGER reject_test_api_key_outbox_event BEFORE INSERT ON identity_access.outbox_events \
         FOR EACH ROW EXECUTE FUNCTION identity_access.reject_test_api_key_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("API key Outbox failure trigger is installed");
    let api_keys_before_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count before Outbox failure is queryable");
    let api_key_outbox_failure = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_api_key_outbox_failure")
        .json(&serde_json::json!({
            "name": "Rollback API key",
            "environment": "test",
            "scopes": ["payment_intents.create"]
        }))
        .send()
        .await
        .expect("API key Outbox failure receives an HTTP response");
    assert_eq!(
        api_key_outbox_failure.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let api_keys_after_outbox_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count after Outbox failure is queryable");
    assert_eq!(
        api_keys_after_outbox_failure, api_keys_before_outbox_failure,
        "API key creation must roll back when its Outbox write fails"
    );
    let failed_api_key_audit_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_access.audit_events WHERE correlation_id='corr_api_key_outbox_failure'",
    )
    .fetch_one(&migration_pool)
    .await
    .expect("failed API key transaction leaves no audit row");
    assert_eq!(failed_api_key_audit_rows, 0);
    sqlx::raw_sql(
        "DROP TRIGGER reject_test_api_key_outbox_event ON identity_access.outbox_events;          DROP FUNCTION identity_access.reject_test_api_key_outbox_event();",
    )
    .execute(&migration_pool)
    .await
    .expect("API key failure trigger is removed");
    let api_key_creation = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_api_key_create")
        .json(&serde_json::json!({
            "name": "Acceptance live key",
            "environment": "live",
            "scopes": ["payment_intents.create", "usage.consume"]
        }))
        .send()
        .await
        .expect(
            "tenant owner can create an API key with payment and quota scopes through BFF and gRPC",
        );
    assert_eq!(api_key_creation.status(), StatusCode::CREATED);
    let api_key_created: serde_json::Value = api_key_creation
        .json()
        .await
        .expect("API key creation returns the one-time secret");
    let api_key_id = Uuid::parse_str(
        api_key_created["key_id"]
            .as_str()
            .expect("API key id is present"),
    )
    .expect("API key id is a UUID");
    let api_key_secret = api_key_created["secret_once"]
        .as_str()
        .expect("one-time API key secret is present");
    assert!(api_key_secret.starts_with("live_"));
    assert_eq!(
        api_key_created["prefix"].as_str(),
        Some(&api_key_secret[..13])
    );
    assert_eq!(api_key_created["environment"], "live");
    let stored_key_digest: Vec<u8> = sqlx::query_scalar(
        "SELECT secret_digest FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
    )
    .bind(created_tenant_id)
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("API key digest is persisted");
    assert_eq!(
        stored_key_digest.as_slice(),
        Sha256::digest(api_key_secret.as_bytes()).as_slice()
    );

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
        api_key_secret: &api_key_secret,
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

    // Exercise the complete seeded tenant-role permission matrix through the
    // real authorization RPC, then verify the exposed tenant/API-key operations.
    let role_matrix = [
        ("developer", &["tenant.read", "api_keys.manage"][..]),
        (
            "tenant_admin",
            &[
                "tenant.read",
                "api_keys.manage",
                "members.manage",
                "tenant.settings.manage",
            ][..],
        ),
        (
            "tenant_owner",
            &[
                "tenant.read",
                "api_keys.manage",
                "members.manage",
                "tenant.settings.manage",
            ][..],
        ),
        ("viewer", &["tenant.read"][..]),
    ];
    for (role, allowed_permissions) in role_matrix {
        sqlx::query(
            "UPDATE identity_access.tenant_memberships SET role=$1 WHERE tenant_id=$2 AND user_id=$3",
        )
        .bind(role)
        .bind(created_tenant_id)
        .bind(registered_user_id)
        .execute(&migration_pool)
        .await
        .expect("tenant membership role can be switched for isolated acceptance");

        for permission in [
            "tenant.read",
            "api_keys.manage",
            "members.manage",
            "tenant.settings.manage",
        ] {
            let decision = authorization_client
                .authorize_tenant_operation(AuthorizeTenantOperationRequest {
                    session_token: session_cookie.clone(),
                    csrf_token: csrf_cookie.clone(),
                    tenant_id: created_tenant_id.to_string(),
                    permission: permission.to_owned(),
                    correlation_id: format!("corr_role_{role}_{permission}"),
                })
                .await;
            if allowed_permissions.contains(&permission) {
                assert!(
                    decision.is_ok(),
                    "role {role} must be allowed permission {permission}: {:?}",
                    decision.err()
                );
            } else {
                assert_eq!(
                    decision
                        .expect_err("ungranted tenant permission is denied")
                        .code(),
                    tonic::Code::PermissionDenied,
                    "role {role} must be denied permission {permission}"
                );
            }
        }

        let tenant_read = client
            .get(format!("http://{http_addr}/v1/tenants/{created_tenant_id}"))
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .send()
            .await
            .expect("tenant read operation returns a response for the current role");
        assert_eq!(
            tenant_read.status(),
            StatusCode::OK,
            "role {role} has tenant.read"
        );

        let api_key_list_for_role = client
            .get(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .send()
            .await
            .expect("API key list operation returns a response for the current role");
        let can_manage_keys = allowed_permissions.contains(&"api_keys.manage");
        assert_eq!(
            api_key_list_for_role.status(),
            if can_manage_keys {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            },
            "role {role} API key list permission is enforced at HTTP boundary"
        );

        let create_key_for_role = client
            .post(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .json(&serde_json::json!({
                "name": format!("Role matrix {role}"),
                "environment": "test",
                "scopes": ["payment_intents.create"]
            }))
            .send()
            .await
            .expect("API key create operation returns a response for the current role");
        assert_eq!(
            create_key_for_role.status(),
            if can_manage_keys {
                StatusCode::CREATED
            } else {
                StatusCode::FORBIDDEN
            },
            "role {role} API key create permission is enforced at HTTP boundary"
        );
        if can_manage_keys {
            let created_for_role: serde_json::Value = create_key_for_role
                .json()
                .await
                .expect("authorized role receives API key metadata");
            let key_id = created_for_role["key_id"]
                .as_str()
                .expect("role matrix key id is present");
            let revoke_for_role = client
                .delete(format!(
                    "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys/{key_id}"
                ))
                .header(header::ORIGIN, &origin)
                .header(
                    header::COOKIE,
                    format!(
                        "payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"
                    ),
                )
                .header("x-csrf-token", &csrf_cookie)
                .send()
                .await
                .expect("authorized role can revoke its own tenant API key");
            assert_eq!(revoke_for_role.status(), StatusCode::NO_CONTENT);
        }
    }
    sqlx::query(
        "UPDATE identity_access.tenant_memberships SET role='tenant_owner' WHERE tenant_id=$1 AND user_id=$2",
    )
    .bind(created_tenant_id)
    .bind(registered_user_id)
    .execute(&migration_pool)
    .await
    .expect("tenant owner role is restored after the role matrix");

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

    // Concurrent tenant registration with the same slug must commit exactly
    // one aggregate and one durable registration event.
    let concurrent_slug = format!("concurrent-slug-{}", Uuid::new_v4().simple());
    let concurrent_slug_document_a = format!(
        "7{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let concurrent_slug_document_b = format!(
        "7{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let concurrent_slug_url = format!("http://{http_addr}/v1/tenants");
    let concurrent_slug_cookie = format!(
        "payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"
    );
    let (concurrent_slug_a, concurrent_slug_b) = tokio::join!(
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_slug_race_a")
            .json(&serde_json::json!({
                "display_name": "Concurrent slug A",
                "slug": concurrent_slug,
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_slug_document_a
            }))
            .send(),
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_slug_race_b")
            .json(&serde_json::json!({
                "display_name": "Concurrent slug B",
                "slug": concurrent_slug,
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_slug_document_b
            }))
            .send()
    );
    let concurrent_slug_a = concurrent_slug_a.expect("first slug race response arrives");
    let concurrent_slug_b = concurrent_slug_b.expect("second slug race response arrives");
    let mut concurrent_slug_statuses = vec![concurrent_slug_a.status(), concurrent_slug_b.status()];
    concurrent_slug_statuses.sort();
    assert_eq!(
        concurrent_slug_statuses,
        vec![StatusCode::CREATED, StatusCode::CONFLICT],
        "concurrent slug registration has one winner and one conflict"
    );
    let concurrent_slug_winner: serde_json::Value =
        if concurrent_slug_a.status() == StatusCode::CREATED {
            concurrent_slug_a
                .json()
                .await
                .expect("winning slug registration returns tenant metadata")
        } else {
            concurrent_slug_b
                .json()
                .await
                .expect("winning slug registration returns tenant metadata")
        };
    let concurrent_slug_winner_id = Uuid::parse_str(
        concurrent_slug_winner["tenant_id"]
            .as_str()
            .expect("winning slug tenant id is present"),
    )
    .expect("winning slug tenant id is a UUID");
    let concurrent_slug_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants WHERE slug=$1")
            .bind(&concurrent_slug)
            .fetch_one(&migration_pool)
            .await
            .expect("concurrent slug row count is queryable");
    assert_eq!(concurrent_slug_rows, 1);
    let concurrent_slug_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
    )
    .bind(concurrent_slug_winner_id)
    .fetch_one(&migration_pool)
    .await
    .expect("concurrent slug Outbox count is queryable");
    assert_eq!(concurrent_slug_events, 1);

    // The legal-identity unique constraint must provide the same guarantee
    // when requests use different slugs but the same normalized document.
    let concurrent_document = format!(
        "6{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let (concurrent_document_a, concurrent_document_b) = tokio::join!(
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_document_race_a")
            .json(&serde_json::json!({
                "display_name": "Concurrent document A",
                "slug": format!("concurrent-document-a-{}", Uuid::new_v4().simple()),
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_document
            }))
            .send(),
        client
            .post(&concurrent_slug_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &concurrent_slug_cookie)
            .header("x-csrf-token", &second_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_document_race_b")
            .json(&serde_json::json!({
                "display_name": "Concurrent document B",
                "slug": format!("concurrent-document-b-{}", Uuid::new_v4().simple()),
                "legal_identification_type": "cc",
                "legal_identification_number": concurrent_document
            }))
            .send()
    );
    let concurrent_document_a =
        concurrent_document_a.expect("first legal identity race response arrives");
    let concurrent_document_b =
        concurrent_document_b.expect("second legal identity race response arrives");
    let mut concurrent_document_statuses = vec![
        concurrent_document_a.status(),
        concurrent_document_b.status(),
    ];
    concurrent_document_statuses.sort();
    assert_eq!(
        concurrent_document_statuses,
        vec![StatusCode::CREATED, StatusCode::CONFLICT],
        "concurrent legal identity registration has one winner and one conflict"
    );
    let concurrent_document_winner: serde_json::Value =
        if concurrent_document_a.status() == StatusCode::CREATED {
            concurrent_document_a
                .json()
                .await
                .expect("winning legal identity registration returns tenant metadata")
        } else {
            concurrent_document_b
                .json()
                .await
                .expect("winning legal identity registration returns tenant metadata")
        };
    let concurrent_document_winner_id = Uuid::parse_str(
        concurrent_document_winner["tenant_id"]
            .as_str()
            .expect("winning legal identity tenant id is present"),
    )
    .expect("winning legal identity tenant id is a UUID");
    let concurrent_document_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.tenants WHERE legal_identification_type='cc' AND legal_identification_number=$1",
    )
    .bind(&concurrent_document)
    .fetch_one(&migration_pool)
    .await
    .expect("concurrent legal identity row count is queryable");
    assert_eq!(concurrent_document_rows, 1);
    let concurrent_document_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND event_type='tenant.registered'",
    )
    .bind(concurrent_document_winner_id)
    .fetch_one(&migration_pool)
    .await
    .expect("concurrent legal identity Outbox count is queryable");
    assert_eq!(concurrent_document_events, 1);

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
    let replay_url = format!(
        "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id={created_tenant_id}"
    );
    let tenant_replay = client
        .get(format!("{replay_url}&cursor=0"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_realtime_tenant_replay")
        .send()
        .await
        .expect("tenant can recover realtime journal by authorized cursor");
    assert_eq!(tenant_replay.status(), StatusCode::OK);
    let replay_body: serde_json::Value = tenant_replay
        .json()
        .await
        .expect("authorized event replay is safe JSON");
    assert!(replay_body["events"].as_array().is_some_and(|events| {
        events
            .iter()
            .any(|event| event["event_type"] == "tenant.registered")
    }));
    let replay_cursor = replay_body["next_cursor"]
        .as_u64()
        .expect("cursor is unsigned");
    let cursor_replay = client
        .get(format!("{replay_url}&cursor={replay_cursor}"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("replay resumes at the acknowledged cursor");
    let cursor_replay_body: serde_json::Value = cursor_replay
        .json()
        .await
        .expect("cursor replay is safe JSON");
    assert_eq!(
        cursor_replay_body["events"].as_array().map(Vec::len),
        Some(0)
    );

    // A cursor ahead of the durable scope cursor is not an empty page: it is
    // an invalid checkpoint and must be rejected so clients do not skip events.
    let future_cursor = client
        .get(format!(
            "{replay_url}&cursor={}",
            replay_cursor.saturating_add(100)
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_realtime_future_cursor")
        .send()
        .await
        .expect("future realtime cursor receives a classified response");
    assert_eq!(future_cursor.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    for invalid_cursor in [
        "-1",
        "not-a-number",
        "9223372036854775808",
        "18446744073709551616",
    ] {
        let invalid_cursor_response = client
            .get(format!("{replay_url}&cursor={invalid_cursor}"))
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .send()
            .await
            .expect("malformed or negative realtime cursor receives an HTTP response");
        assert_eq!(
            invalid_cursor_response.status(),
            StatusCode::BAD_REQUEST,
            "invalid cursor {invalid_cursor} must be rejected at the HTTP boundary"
        );
    }
    let invalid_realtime_scope = client
        .get(format!(
            "http://{http_addr}/v1/realtime/events?audience=unknown&cursor=0"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("unsupported realtime audience receives an HTTP response");
    assert_eq!(invalid_realtime_scope.status(), StatusCode::BAD_REQUEST);
    let malformed_realtime_tenant = client
        .get(format!(
            "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id=not-a-uuid&cursor=0"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("malformed realtime tenant scope receives an HTTP response");
    assert_eq!(malformed_realtime_tenant.status(), StatusCode::BAD_REQUEST);

    let foreign_tenant_replay = client
        .get(format!(
            "http://{http_addr}/v1/realtime/events?audience=tenant&tenant_id={}&cursor=0",
            Uuid::new_v4()
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant realtime audience cannot cross tenant boundary");
    assert_eq!(foreign_tenant_replay.status(), StatusCode::FORBIDDEN);
    let status_event_message = stream
        .get_last_raw_message_by_subject(&format!(
            "{}.tenant.status.changed",
            event_topology.subject_prefix
        ))
        .await
        .expect("JetStream durably stores administrative status events");
    platform_realtime::infrastructure::grpc::realtime_gateway_api::persist_and_publish(
        &runtime_pool,
        &mut realtime_redis,
        &status_event_message.payload,
    )
    .await
    .expect("status transition is journaled for realtime replay");
    let platform_replay = client
        .get(format!("http://{http_addr}/v1/realtime/events?audience=platform&cursor=0"))
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .header("x-correlation-id", "corr_realtime_platform_replay")
        .send()
        .await
        .expect("platform operator recovers administrative events through authorized BFF");
    assert_eq!(platform_replay.status(), StatusCode::OK);
    let platform_replay_body: serde_json::Value = platform_replay
        .json()
        .await
        .expect("platform event replay is safe JSON");
    assert!(
        platform_replay_body["events"]
            .as_array()
            .is_some_and(|events| {
                events
                    .iter()
                    .any(|event| event["event_type"] == "tenant.status.changed")
            })
    );
    // Keep an authenticated realtime subscription open while the operator
    // session is revoked. The stream must re-check authorization and terminate.
    let mut realtime_subscription_client = realtime_acceptance_client.clone();
    let realtime_subscription = realtime_subscription_client
        .subscribe_events(SubscribeEventsRequest {
            session_token: operator_session_cookie.clone(),
            csrf_token: operator_csrf_cookie.clone(),
            audience: "platform".to_owned(),
            tenant_id: String::new(),
            after_cursor: 0,
            correlation_id: "corr_realtime_revocation_subscription".to_owned(),
        })
        .await
        .expect("authorized operator can subscribe to platform realtime")
        .into_inner();

    let operator_logout = client
        .delete(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", &operator_csrf_cookie)
        .header("x-correlation-id", "corr_operator_logout")
        .send()
        .await
        .expect("operator logout response arrives");
    assert_eq!(operator_logout.status(), StatusCode::NO_CONTENT);
    let mut revoked_subscription = realtime_subscription;
    let realtime_revocation_status = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match revoked_subscription.message().await {
                Err(status) => break status,
                Ok(Some(_event)) => continue,
                Ok(None) => panic!("realtime subscription must report authorization revocation"),
            }
        }
    })
    .await
    .expect("realtime subscription revalidates operator authorization within 20 seconds");
    assert_eq!(
        realtime_revocation_status.code(),
        tonic::Code::Unauthenticated
    );

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

    let mut tenant_realtime_subscription_client = realtime_acceptance_client.clone();
    let mut tenant_realtime_subscription = tenant_realtime_subscription_client
        .subscribe_events(SubscribeEventsRequest {
            session_token: session_cookie.clone(),
            csrf_token: csrf_cookie.clone(),
            audience: "tenant".to_owned(),
            tenant_id: created_tenant_id.to_string(),
            after_cursor: replay_cursor,
            correlation_id: "corr_realtime_tenant_revocation_subscription".to_owned(),
        })
        .await
        .expect("authorized tenant can subscribe to its realtime stream")
        .into_inner();

    let logout = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .header("x-correlation-id", "corr_identity_logout")
        .send()
        .await
        .expect("valid logout response arrives");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let tenant_revocation_status = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match tenant_realtime_subscription.message().await {
                Err(status) => break status,
                Ok(Some(_event)) => continue,
                Ok(None) => {
                    panic!("tenant realtime subscription must report authorization revocation")
                }
            }
        }
    })
    .await
    .expect("tenant realtime subscription revalidates session revocation");
    assert_eq!(
        tenant_revocation_status.code(),
        tonic::Code::Unauthenticated
    );
    let tenant_logout_audited: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'identity.session.revoked' AND correlation_id = 'corr_identity_logout')",
    )
    .bind(registered_user_id)
    .fetch_one(&migration_pool)
    .await
    .expect("tenant session revocation audit is queryable");
    assert!(
        tenant_logout_audited,
        "tenant session revocation is auditable"
    );
    let replay = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
        .send()
        .await
        .expect("revoked session replay is rejected");
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);

    for attempt in 0..9 {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, &origin)
            .header(
                "x-correlation-id",
                format!("corr_identity_rate_limit_{attempt}"),
            )
            .header("traceparent", &traceparent)
            .json(&serde_json::json!({"email": email, "password": "incorrect-password"}))
            .send()
            .await
            .expect("rate limited login response arrives");
        let expected_status = if attempt == 8 {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::UNAUTHORIZED
        };
        assert_eq!(
            response.status(),
            expected_status,
            "unexpected status for rate limit attempt {attempt}"
        );
        if attempt == 8 {
            let rate_limit_error: serde_json::Value =
                response.json().await.expect("safe rate limit envelope");
            assert_eq!(rate_limit_error["error"]["code"], "identity_rate_limited");
            assert_eq!(
                rate_limit_error["error"]["correlation_id"],
                "corr_identity_rate_limit_8"
            );
        }
    }

    let mut isolated_ip_counter = redis_manager.clone();
    let _: u64 = redis::cmd("DEL")
        .arg(&ip_key)
        .query_async(&mut isolated_ip_counter)
        .await
        .expect("test-owned per-IP counter is reset before threshold acceptance");
    for attempt in 0..51 {
        let response = client
            .post(format!("http://{http_addr}/v1/identity/session"))
            .header(header::ORIGIN, &origin)
            .header(
                "x-correlation-id",
                format!("corr_identity_ip_limit_{attempt}"),
            )
            .json(&serde_json::json!({
                "email": format!("unknown_{attempt}@example.test"),
                "password": "incorrect-password"
            }))
            .send()
            .await
            .expect("per-IP rate limit response arrives");
        let expected_status = if attempt == 50 {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::UNAUTHORIZED
        };
        assert_eq!(
            response.status(),
            expected_status,
            "unexpected status for per-IP rate limit attempt {attempt}"
        );
        if attempt == 50 {
            let rate_limit_error: serde_json::Value =
                response.json().await.expect("safe per-IP limit envelope");
            assert_eq!(rate_limit_error["error"]["code"], "identity_rate_limited");
            assert_eq!(
                rate_limit_error["error"]["correlation_id"],
                "corr_identity_ip_limit_50"
            );
        }
    }

    let rate_limit_events = captured_events.lock().expect("event capture lock").clone();
    let rate_limit_module_failure = rate_limit_events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("grpc.module.request.failed"))
                && event
                    .get("error_code")
                    .is_some_and(|code| code.contains("identity_rate_limited"))
                && event
                    .get("correlation_id")
                    .is_some_and(|value| value.contains("corr_identity_rate_limit_8"))
        })
        .expect("causal rate limit event is recorded");
    let rate_limit_http_closure = rate_limit_events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("http.request.completed"))
                && event
                    .get("correlation_id")
                    .is_some_and(|value| value.contains("corr_identity_rate_limit_8"))
        })
        .expect("rate limit HTTP closure event is recorded");
    for field in ["correlation_id", "trace_id"] {
        assert_eq!(
            rate_limit_module_failure.get(field),
            rate_limit_http_closure.get(field)
        );
    }
    assert!(
        rate_limit_http_closure
            .get("error_code")
            .is_some_and(|code| code.contains("identity_rate_limited"))
    );
    assert!(
        rate_limit_http_closure
            .get("error_category")
            .is_some_and(|category| category.contains("rate_limit"))
    );

    // The public registration endpoint is limited per normalized account and per peer IP.
    // Invalid passwords still consume an attempt, preventing cheap validation probes from
    // bypassing the protection around the expensive password-hashing path.
    // Use a dedicated loopback source IP so earlier registration requests in this
    // end-to-end scenario cannot consume the IP budget being asserted here.
    let rate_limit_source_id = Uuid::new_v4();
    let rate_limit_source_bytes = rate_limit_source_id.as_bytes();
    let rate_limit_source_ip = std::net::Ipv4Addr::new(
        127,
        rate_limit_source_bytes[0],
        rate_limit_source_bytes[1],
        rate_limit_source_bytes[2].max(1),
    );
    let rate_limit_client = Client::builder()
        .local_address(std::net::IpAddr::V4(rate_limit_source_ip))
        .default_headers(default_headers)
        .build()
        .expect("isolated rate-limit HTTP client builds");
    let limited_email = format!("limited_{}@example.test", Uuid::new_v4().simple());
    for attempt in 0..5 {
        let response = rate_limit_client
            .post(format!("http://{http_addr}/v1/identity/register"))
            .header(header::ORIGIN, &origin)
            .json(&serde_json::json!({
                "email": limited_email,
                "password": "long-lowercase-password"
            }))
            .send()
            .await
            .expect("registration attempt reaches the BFF");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "attempt {attempt} is below the per-account registration limit"
        );
    }
    let account_limited = rate_limit_client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .json(&serde_json::json!({
            "email": limited_email,
            "password": "long-lowercase-password"
        }))
        .send()
        .await
        .expect("account-limited registration response arrives");
    assert_eq!(account_limited.status(), StatusCode::TOO_MANY_REQUESTS);

    // A separate set of accounts from the same peer proves the independent IP ceiling.
    for attempt in 6..20 {
        let response = rate_limit_client
            .post(format!("http://{http_addr}/v1/identity/register"))
            .header(header::ORIGIN, &origin)
            .json(&serde_json::json!({
                "email": format!("ip_limit_{attempt}_{}@example.test", Uuid::new_v4().simple()),
                "password": "long-lowercase-password"
            }))
            .send()
            .await
            .expect("IP-limited registration attempt reaches the BFF");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "IP attempt {attempt} is below the per-IP registration limit"
        );
    }
    let ip_limited = rate_limit_client
        .post(format!("http://{http_addr}/v1/identity/register"))
        .header(header::ORIGIN, &origin)
        .json(&serde_json::json!({
            "email": format!("ip_limit_final_{}@example.test", Uuid::new_v4().simple()),
            "password": "long-lowercase-password"
        }))
        .send()
        .await
        .expect("IP-limited registration response arrives");
    assert_eq!(ip_limited.status(), StatusCode::TOO_MANY_REQUESTS);

    let event_values: Vec<String> = captured_events
        .lock()
        .expect("event capture lock")
        .iter()
        .flat_map(|event| event.values().cloned())
        .collect();
    assert!(!event_values.iter().any(|value| value.contains(&password)));
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&operator_password))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&session_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&csrf_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&operator_session_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&operator_csrf_cookie))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&legal_identification_number))
    );
    assert!(
        !event_values
            .iter()
            .any(|value| value.contains(&second_legal_identification_number))
    );

    // Exercise operator grants and real tenant administration endpoints for
    // platform_owner, platform_security, and platform_support.
    let operator_role_matrix = [
        (
            "platform_owner",
            &[
                "tenants.read",
                "tenants.verify",
                "tenants.activate",
                "tenants.suspend",
                "tenants.close",
                "operators.bootstrap",
            ][..],
        ),
        (
            "platform_security",
            &[
                "tenants.read",
                "tenants.verify",
                "tenants.activate",
                "tenants.suspend",
                "tenants.close",
            ][..],
        ),
        ("platform_support", &["tenants.read"][..]),
    ];
    for (role, allowed_permissions) in operator_role_matrix {
        sqlx::query("UPDATE identity_access.operator_users SET role=$1 WHERE id=$2")
            .bind(role)
            .bind(operator_id)
            .execute(&migration_pool)
            .await
            .expect("operator role can be switched for isolated acceptance");
        for permission in [
            "tenants.read",
            "tenants.verify",
            "tenants.activate",
            "tenants.suspend",
            "tenants.close",
            "operators.bootstrap",
        ] {
            let decision = authorization_client
                .authorize_operator_operation(AuthorizeOperatorOperationRequest {
                    session_token: matrix_operator_session_cookie.clone(),
                    csrf_token: matrix_operator_csrf_cookie.clone(),
                    permission: permission.to_owned(),
                    correlation_id: format!("corr_operator_role_{role}_{permission}"),
                })
                .await;
            if allowed_permissions.contains(&permission) {
                assert!(
                    decision.is_ok(),
                    "role {role} must be allowed permission {permission}"
                );
            } else {
                assert_eq!(
                    decision
                        .expect_err("ungranted operator permission is denied")
                        .code(),
                    tonic::Code::PermissionDenied,
                    "role {role} must be denied permission {permission}"
                );
            }
        }
        let operator_tenant_list = client
            .get(format!("http://{http_addr}/v1/platform/tenants?limit=10&offset=0"))
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .send()
            .await
            .expect("operator tenant listing responds for each role");
        assert_eq!(
            operator_tenant_list.status(),
            StatusCode::OK,
            "all operator roles have tenants.read"
        );
        let operator_tenant_detail = client
            .get(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}"
            ))
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .send()
            .await
            .expect("operator tenant detail endpoint responds for each role");
        assert_eq!(
            operator_tenant_detail.status(),
            StatusCode::OK,
            "operator role {role} can read tenant details"
        );
    }

    // platform_support may read tenants but must not suspend them. Verify that
    // the denial leaves both status and version unchanged.
    let support_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Support role must not change tenant lifecycle",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("support lifecycle mutation receives a response");
    assert_eq!(support_status_change.status(), StatusCode::FORBIDDEN);
    let status_after_support_denial: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("tenant state after support denial is queryable");
    assert_eq!(status_after_support_denial, ("active".to_owned(), 2));

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform security role is selected for lifecycle acceptance");
    let security_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Authorized platform security suspension",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("platform security can suspend a tenant");
    assert_eq!(security_status_change.status(), StatusCode::OK);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform owner role is restored for lifecycle acceptance");
    let owner_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Authorized platform owner reactivation",
            "expected_version": 3
        }))
        .send()
        .await
        .expect("platform owner can reactivate a suspended tenant");
    assert_eq!(owner_status_change.status(), StatusCode::OK);

    let owner_suspend_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Authorized platform owner suspension",
            "expected_version": 4
        }))
        .send()
        .await
        .expect("platform owner can suspend a tenant");
    assert_eq!(owner_suspend_status_change.status(), StatusCode::OK);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform security role is selected for lifecycle reactivation");
    let security_reactivation = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Authorized platform security reactivation",
            "expected_version": 5
        }))
        .send()
        .await
        .expect("platform security can reactivate a suspended tenant");
    assert_eq!(security_reactivation.status(), StatusCode::OK);
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform owner role is restored for remaining lifecycle acceptance");

    // Invalid reasons and stale optimistic-lock versions must be rejected
    // before changing the tenant aggregate or its version. Read the committed
    // version instead of duplicating assumptions about prior transitions.
    let tenant_before_invalid_transitions: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("tenant state before invalid transitions is queryable");
    assert_eq!(tenant_before_invalid_transitions.0, "active");
    let current_status_version = tenant_before_invalid_transitions.1;
    assert!(current_status_version > 0);

    let missing_status_reason = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .header("x-correlation-id", "corr_tenant_status_missing_reason")
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": " ",
            "expected_version": current_status_version
        }))
        .send()
        .await
        .expect("status transition without a meaningful reason is rejected");
    assert_eq!(missing_status_reason.status(), StatusCode::BAD_REQUEST);
    let invalid_reasons = vec!["ab".to_owned(), "r".repeat(501)];
    for invalid_reason in invalid_reasons {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .json(&serde_json::json!({
                "status": "suspended",
                "reason": invalid_reason,
                "expected_version": current_status_version
            }))
            .send()
            .await
            .expect("out-of-range lifecycle reason receives an HTTP response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let stale_status_version = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .header("x-correlation-id", "corr_tenant_status_stale_version")
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "This stale transition must not be applied",
            "expected_version": current_status_version - 1
        }))
        .send()
        .await
        .expect("stale tenant status version is rejected");
    assert_eq!(stale_status_version.status(), StatusCode::CONFLICT);

    let zero_expected_version = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "A version must be positive",
            "expected_version": -1
        }))
        .send()
        .await
        .expect("negative expected version is rejected as invalid input");
    assert_eq!(zero_expected_version.status(), StatusCode::BAD_REQUEST);

    let wrong_version_type = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "The version must be an integer",
            "expected_version": "not-an-integer"
        }))
        .send()
        .await
        .expect("wrong expected_version JSON type is rejected");
    assert_eq!(
        wrong_version_type.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let invalid_status_dto_types = [
        serde_json::json!({
            "status": 7,
            "reason": "Status must be a string",
            "expected_version": current_status_version
        }),
        serde_json::json!({
            "status": "suspended",
            "reason": 7,
            "expected_version": current_status_version
        }),
    ];
    for (index, invalid_body) in invalid_status_dto_types.into_iter().enumerate() {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("wrong lifecycle DTO field type receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong lifecycle DTO type case {index} must be rejected"
        );
    }

    let invalid_status_value = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "unknown-state",
            "reason": "Unknown states must not be accepted",
            "expected_version": current_status_version
        }))
        .send()
        .await
        .expect("unknown lifecycle state is rejected");
    assert_eq!(invalid_status_value.status(), StatusCode::BAD_REQUEST);

    for invalid_query in ["limit=0", "limit=101", "offset=-1", "limit=not-a-number"] {
        let invalid_list = client
            .get(format!("http://{http_addr}/v1/platform/tenants?{invalid_query}"))
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .send()
            .await
            .expect("invalid tenant-list pagination receives an HTTP response");
        assert_eq!(
            invalid_list.status(),
            StatusCode::BAD_REQUEST,
            "invalid pagination query {invalid_query} must be rejected"
        );
    }
    let oversized_tenant_query = client
        .get(format!(
            "http://{http_addr}/v1/platform/tenants?query={}",
            "q".repeat(121)
        ))
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .send()
        .await
        .expect("oversized tenant search query receives an HTTP response");
    assert_eq!(oversized_tenant_query.status(), StatusCode::BAD_REQUEST);

    let tenants_before_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count before invalid DTO checks is queryable");
    let invalid_tenant_values = [
        serde_json::json!({"display_name": "A", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "a".repeat(65), "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "passport", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "-12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "1".repeat(21)}),
    ];
    for (index, invalid_body) in invalid_tenant_values.into_iter().enumerate() {
        let response = client
            .post(format!("http://{http_addr}/v1/tenants"))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
            )
            .header("x-csrf-token", &second_csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("invalid tenant registration values receive an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "invalid tenant registration value case {index} must fail validation"
        );
    }
    let invalid_tenant_types = [
        serde_json::json!({"display_name": 7, "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": 7, "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": 7, "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": 7}),
    ];
    for (index, invalid_body) in invalid_tenant_types.into_iter().enumerate() {
        let response = client
            .post(format!("http://{http_addr}/v1/tenants"))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
            )
            .header("x-csrf-token", &second_csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("incorrect tenant DTO field type receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong tenant DTO type case {index} must be rejected by the HTTP extractor"
        );
    }
    let tenants_after_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count after invalid DTO checks is queryable");
    assert_eq!(
        tenants_after_invalid_dtos, tenants_before_invalid_dtos,
        "invalid tenant values and types must not create tenant rows"
    );

    let invalid_tenant_dto = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .header("x-csrf-token", &second_csrf_cookie)
        .json(&serde_json::json!({
            "display_name": "Invalid tenant DTO",
            "slug": 42,
            "legal_identification_type": "nit",
            "legal_identification_number": "123456"
        }))
        .send()
        .await
        .expect("wrong tenant DTO field type is rejected");
    assert_eq!(
        invalid_tenant_dto.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let tenant_after_invalid_transitions: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("tenant state after invalid transitions is queryable");
    assert_eq!(
        tenant_after_invalid_transitions, tenant_before_invalid_transitions,
        "invalid reason and stale version leave tenant state unchanged"
    );

    // Exercise activation and both close transitions on a separate tenant.
    // platform_support must be denied; platform_security may activate and close.
    let second_tenant_initial_state: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(second_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("second tenant lifecycle state is queryable");
    assert_eq!(
        second_tenant_initial_state,
        ("pending_verification".to_owned(), 1)
    );
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_support' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform support role is selected for activation denial");
    let support_activation = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Support role must not activate a tenant",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("support activation receives an HTTP response");
    assert_eq!(support_activation.status(), StatusCode::FORBIDDEN);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform security role is selected for activation");
    let security_activation = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "active",
            "reason": "Authorized platform security verification",
            "expected_version": 1
        }))
        .send()
        .await
        .expect("platform security can activate a verified tenant");
    assert_eq!(security_activation.status(), StatusCode::OK);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_support' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform support role is selected for close denial");
    let support_close = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "closing",
            "reason": "Support role must not close a tenant",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("support close receives an HTTP response");
    assert_eq!(support_close.status(), StatusCode::FORBIDDEN);

    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform owner role is selected for tenant close");
    let owner_closing = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "closing",
            "reason": "Authorized platform security close",
            "expected_version": 2
        }))
        .send()
        .await
        .expect("platform owner can start tenant close");
    assert_eq!(owner_closing.status(), StatusCode::OK);
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_security' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform security role is selected to finish tenant close");
    let security_closed = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{second_tenant_id}/status"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "closed",
            "reason": "Authorized platform security final close",
            "expected_version": 3
        }))
        .send()
        .await
        .expect("platform security can finish tenant close");
    assert_eq!(security_closed.status(), StatusCode::OK);
    let second_tenant_closed_state: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(second_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("closed tenant lifecycle state is queryable");
    assert_eq!(second_tenant_closed_state, ("closed".to_owned(), 4));
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform owner role is restored after lifecycle matrix");

    // Mutating HTTP endpoints must reject missing CSRF tokens or untrusted origins
    // before reaching a module or changing durable state.
    let tenants_before_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count before CSRF rejection is queryable");
    let csrf_rejected_tenant_create = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .json(&serde_json::json!({
            "display_name": "Must not be created",
            "slug": format!("csrf-rejected-{}", Uuid::new_v4().simple()),
            "legal_identification_type": "nit",
            "legal_identification_number": format!("8{:018}", Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000)
        }))
        .send()
        .await
        .expect("missing CSRF token receives an HTTP response");
    assert_eq!(csrf_rejected_tenant_create.status(), StatusCode::FORBIDDEN);
    let tenants_after_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(&migration_pool)
            .await
            .expect("tenant count after CSRF rejection is queryable");
    assert_eq!(tenants_after_csrf_rejection, tenants_before_csrf_rejection);

    let keys_before_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count before CSRF rejection is queryable");
    let csrf_rejected_api_key = client
        .post(format!(
            "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
        ))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .json(&serde_json::json!({
            "name": "Must not be created",
            "environment": "test",
            "scopes": ["payment_intents.create"]
        }))
        .send()
        .await
        .expect("missing CSRF token rejects API key creation");
    assert_eq!(csrf_rejected_api_key.status(), StatusCode::FORBIDDEN);
    let keys_after_csrf_rejection: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_access.api_keys")
            .fetch_one(&migration_pool)
            .await
            .expect("API key count after CSRF rejection is queryable");
    assert_eq!(keys_after_csrf_rejection, keys_before_csrf_rejection);

    let csrf_rejected_status_change = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, "https://attacker.example")
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "Must not be applied",
            "expected_version": tenant_admin_status_body["status_version"]
        }))
        .send()
        .await
        .expect("untrusted Origin rejects status mutation");
    assert_eq!(csrf_rejected_status_change.status(), StatusCode::FORBIDDEN);

    let csrf_rejected_tenant_logout = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("missing CSRF token rejects tenant logout");
    assert_eq!(csrf_rejected_tenant_logout.status(), StatusCode::FORBIDDEN);

    let csrf_rejected_operator_logout = client
        .delete(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"),
        )
        .send()
        .await
        .expect("missing CSRF token rejects operator logout");
    assert_eq!(
        csrf_rejected_operator_logout.status(),
        StatusCode::FORBIDDEN
    );

    // Expired and revoked sessions are rejected at the HTTP boundary.
    // The second tenant session has not been revoked; expire it explicitly so
    // this assertion distinguishes expiration from the separate logout checks.
    let expired_session_digest = Sha256::digest(second_session_cookie.as_bytes());
    sqlx::query(
        "UPDATE identity_access.sessions SET expires_at=now()-interval '1 second' WHERE token_digest=$1",
    )
    .bind(expired_session_digest.as_slice())
    .execute(&migration_pool)
    .await
    .expect("active second tenant session is marked expired in isolated PostgreSQL");
    let expired_session_response = client
        .get(format!("http://{http_addr}/v1/identity/session"))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .send()
        .await
        .expect("expired session receives an HTTP response");
    assert_eq!(expired_session_response.status(), StatusCode::UNAUTHORIZED);
    let expired_session_api_key_access = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{second_tenant_id}/api-keys"
        ))
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .send()
        .await
        .expect("expired tenant session cannot access API key resources");
    assert_eq!(
        expired_session_api_key_access.status(),
        StatusCode::UNAUTHORIZED
    );

    // Two lifecycle mutations using the same expected version must serialize:
    // one commits, the other receives a version conflict regardless of which
    // state transition wins the row lock.
    sqlx::query("UPDATE identity_access.operator_users SET role='platform_owner' WHERE id=$1")
        .bind(operator_id)
        .execute(&migration_pool)
        .await
        .expect("platform owner role is restored for concurrent lifecycle acceptance");
    let race_status_url =
        format!("http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status");
    let race_cookie = format!(
        "payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"
    );
    let (suspend_race, close_race) = tokio::join!(
        client
            .post(&race_status_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &race_cookie)
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_status_race_suspend")
            .json(&serde_json::json!({
                "status": "suspended",
                "reason": "Concurrent optimistic-lock suspension",
                "expected_version": current_status_version
            }))
            .send(),
        client
            .post(&race_status_url)
            .header(header::ORIGIN, &origin)
            .header(header::COOKIE, &race_cookie)
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .header("x-correlation-id", "corr_tenant_status_race_close")
            .json(&serde_json::json!({
                "status": "closing",
                "reason": "Concurrent optimistic-lock close",
                "expected_version": current_status_version
            }))
            .send()
    );
    let suspend_race = suspend_race.expect("concurrent suspension response arrives");
    let close_race = close_race.expect("concurrent close response arrives");
    let mut race_statuses = vec![suspend_race.status(), close_race.status()];
    race_statuses.sort();
    assert_eq!(
        race_statuses,
        vec![StatusCode::OK, StatusCode::CONFLICT],
        "exactly one concurrent tenant status mutation may commit"
    );
    let race_conflict = if suspend_race.status() == StatusCode::CONFLICT {
        suspend_race
    } else {
        close_race
    };
    let race_conflict_body: serde_json::Value = race_conflict
        .json()
        .await
        .expect("optimistic-lock race returns a safe conflict envelope");
    assert_eq!(
        race_conflict_body["error"]["code"], "tenant_state_conflict",
        "the losing request must be classified as a stale-version conflict"
    );
    let race_final_state: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(&migration_pool)
            .await
            .expect("concurrent lifecycle final state is queryable");
    assert!(
        matches!(race_final_state.0.as_str(), "suspended" | "closing"),
        "one of the two requested transitions must win"
    );
    assert_eq!(race_final_state.1, current_status_version + 1);
    let race_history_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.status_history WHERE tenant_id=$1 AND version=$2",
    )
    .bind(created_tenant_id)
    .bind(current_status_version + 1)
    .fetch_one(&migration_pool)
    .await
    .expect("single concurrent status-history row is queryable");
    assert_eq!(race_history_count, 1);
    let race_outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.outbox_events WHERE aggregate_id=$1 AND aggregate_version=$2 AND event_type='tenant.status.changed'",
    )
    .bind(created_tenant_id)
    .bind(current_status_version + 1)
    .fetch_one(&migration_pool)
    .await
    .expect("single concurrent status Outbox event is queryable");
    assert_eq!(race_outbox_count, 1);

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

    // Expired Outbox leases are reclaimable, and fencing tokens prevent a
    // previous worker from acknowledging an event after another worker owns it.
    for schema in ["identity_access", "tenant_registry"] {
        if schema == "identity_access" {
            sqlx::query(
                "UPDATE identity_access.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL",
            )
            .execute(&migration_pool)
            .await
            .expect("identity pending events are deferred for lease acceptance");
        } else {
            sqlx::query(
                "UPDATE tenant_registry.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL",
            )
            .execute(&migration_pool)
            .await
            .expect("tenant pending events are deferred for lease acceptance");
        }
        let event_id = Uuid::new_v4();
        let aggregate_id = Uuid::new_v4();
        let stale_lease = Uuid::new_v4();
        let insert_expired = if schema == "identity_access" {
            sqlx::query(
                "INSERT INTO identity_access.outbox_events                  (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id,available_at,lease_token,lease_until,attempts)                  VALUES ($1,$2,1,'tenant.status.changed',1,'tenant_and_platform',$3,$4,now()-interval '1 second',$5,now()-interval '1 second',2)",
            )
        } else {
            sqlx::query(
                "INSERT INTO tenant_registry.outbox_events                  (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id,available_at,lease_token,lease_until,attempts)                  VALUES ($1,$2,1,'tenant.status.changed',1,'tenant_and_platform',$3,$4,now()-interval '1 second',$5,now()-interval '1 second',2)",
            )
        };
        insert_expired
            .bind(event_id)
            .bind(aggregate_id)
            .bind(serde_json::json!({"tenant_id": created_tenant_id}))
            .bind(format!("corr_{schema}_expired_lease"))
            .bind(stale_lease)
            .execute(&migration_pool)
            .await
            .expect("expired synthetic Outbox lease is inserted");
        let (claimed_event_id, new_lease) = if schema == "identity_access" {
            let claimed =
                platform_identity_access::infrastructure::persistence::claim_outbox_events(
                    &runtime_pool,
                    32,
                )
                .await
                .expect("identity Outbox worker reclaims an expired lease");
            assert_eq!(
                claimed.len(),
                1,
                "only the expired {schema} event is eligible"
            );
            (claimed[0].event_id, claimed[0].lease_token)
        } else {
            let claimed =
                platform_tenant_registry::infrastructure::persistence::claim_outbox_events(
                    &runtime_pool,
                    32,
                )
                .await
                .expect("tenant Outbox worker reclaims an expired lease");
            assert_eq!(
                claimed.len(),
                1,
                "only the expired {schema} event is eligible"
            );
            (claimed[0].event_id, claimed[0].lease_token)
        };
        assert_eq!(claimed_event_id, event_id);
        assert_ne!(new_lease, stale_lease);
        let old_worker_acknowledged = if schema == "identity_access" {
            platform_identity_access::infrastructure::persistence::mark_outbox_published(
                &runtime_pool,
                event_id,
                stale_lease,
            )
            .await
            .expect("identity stale lease acknowledgement is checked")
        } else {
            platform_tenant_registry::infrastructure::persistence::mark_outbox_published(
                &runtime_pool,
                event_id,
                stale_lease,
            )
            .await
            .expect("tenant stale lease acknowledgement is checked")
        };
        assert!(!old_worker_acknowledged, "stale {schema} lease cannot ack");
        let new_worker_acknowledged = if schema == "identity_access" {
            platform_identity_access::infrastructure::persistence::mark_outbox_published(
                &runtime_pool,
                event_id,
                new_lease,
            )
            .await
            .expect("identity current lease acknowledgement is checked")
        } else {
            platform_tenant_registry::infrastructure::persistence::mark_outbox_published(
                &runtime_pool,
                event_id,
                new_lease,
            )
            .await
            .expect("tenant current lease acknowledgement is checked")
        };
        assert!(new_worker_acknowledged, "current {schema} lease can ack");
        let attempts: i32 = if schema == "identity_access" {
            sqlx::query_scalar(
                "SELECT attempts FROM identity_access.outbox_events WHERE event_id=$1",
            )
            .bind(event_id)
            .fetch_one(&migration_pool)
            .await
        } else {
            sqlx::query_scalar(
                "SELECT attempts FROM tenant_registry.outbox_events WHERE event_id=$1",
            )
            .bind(event_id)
            .fetch_one(&migration_pool)
            .await
        }
        .expect("reclaimed Outbox attempt count is queryable");
        assert_eq!(attempts, 3);
    }

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
