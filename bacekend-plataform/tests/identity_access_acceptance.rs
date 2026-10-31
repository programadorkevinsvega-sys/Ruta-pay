use async_nats::jetstream;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use platform_contracts::{
    identity_access::{
        AuthorizeOperatorOperationRequest, identity_access_client::IdentityAccessClient,
    },
    realtime::realtime_gateway_client::RealtimeGatewayClient as RealtimeGatewayGrpcClient,
    runtime::runtime_probe_client::RuntimeProbeClient,
    tenant_registry::tenant_registry_client::TenantRegistryClient as TenantRegistryGrpcClient,
};
use platform_identity_access::application::operator_bootstrap::OperatorBootstrapService;
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use reqwest::{Client, StatusCode, header};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};
use tracing::{
    Event,
    field::{Field, Visit},
};
use tracing_subscriber::{
    layer::{Context, Layer},
    prelude::*,
    registry::LookupSpan,
};
use uuid::Uuid;

#[derive(Clone, Default)]
struct EventCapture(Arc<Mutex<Vec<HashMap<String, String>>>>);

impl<S> Layer<S> for EventCapture
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut fields = HashMap::new();
        event.record(&mut FieldCapture(&mut fields));
        if let Some(span) = context.lookup_current() {
            fields.insert("current_span".to_owned(), span.name().to_owned());
        }
        self.0.lock().expect("event capture lock").push(fields);
    }
}

struct FieldCapture<'a>(&'a mut HashMap<String, String>);

fn is_stream_not_found(error: async_nats::jetstream::context::GetStreamError) -> bool {
    matches!(
        error.kind(),
        async_nats::jetstream::context::GetStreamErrorKind::JetStream(jetstream_error)
            if jetstream_error.kind() == async_nats::jetstream::ErrorCode::STREAM_NOT_FOUND
    )
}

struct EventStreamCleanup {
    jetstream: jetstream::Context,
    namespace: String,
    armed: bool,
}

struct AcceptanceDatabaseCleanup {
    admin_pool: sqlx::PgPool,
    database_name: String,
    runtime_role: String,
    armed: bool,
}

impl AcceptanceDatabaseCleanup {
    async fn cleanup(&mut self) {
        Self::drop_owned_resources(&self.admin_pool, &self.database_name, &self.runtime_role)
            .await
            .expect("acceptance database and role are removed");
        self.armed = false;
    }

    async fn drop_owned_resources(
        admin_pool: &sqlx::PgPool,
        database_name: &str,
        runtime_role: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
        )
        .bind(database_name)
        .execute(admin_pool)
        .await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{database_name}\""
        )))
        .execute(admin_pool)
        .await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP ROLE IF EXISTS \"{runtime_role}\""
        )))
        .execute(admin_pool)
        .await?;
        Ok(())
    }
}

impl Drop for AcceptanceDatabaseCleanup {
    fn drop(&mut self) {
        if self.armed {
            let admin_pool = self.admin_pool.clone();
            let database_name = self.database_name.clone();
            let runtime_role = self.runtime_role.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                runtime.block_on(async move {
                    Self::drop_owned_resources(&admin_pool, &database_name, &runtime_role)
                        .await
                        .expect("panic cleanup removes only this acceptance database and role");
                })
            });
        }
    }
}

impl EventStreamCleanup {
    async fn cleanup(&mut self) {
        delete_e2e_stream(&self.jetstream, &self.namespace)
            .await
            .expect("only the integration test stream is deleted");
        self.armed = false;
    }
}

impl Drop for EventStreamCleanup {
    fn drop(&mut self) {
        if self.armed {
            let jetstream = self.jetstream.clone();
            let namespace = self.namespace.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                runtime.block_on(async move {
                    delete_e2e_stream(&jetstream, &namespace)
                        .await
                        .expect("panic cleanup deletes only this integration stream");
                })
            });
        }
    }
}

impl Visit for FieldCapture<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

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
        "GRANT USAGE ON SCHEMA identity_access, tenant_registry, realtime TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("runtime role receives schema usage");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA identity_access, tenant_registry, realtime TO \"{runtime_role}\"")))
        .execute(&migration_pool)
        .await
        .expect("runtime role receives owned-module table privileges");
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

    let redis_client = redis::Client::open(redis_url).expect("Redis URL parses");
    let redis_manager = redis::aio::ConnectionManager::new(redis_client.clone())
        .await
        .expect("real development Redis accepts a connection");
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.expect("gRPC bind");
    let grpc_addr = grpc_listener.local_addr().expect("gRPC address");
    let module_identity_client = IdentityAccessClient::new(
        tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
            .expect("identity module endpoint parses")
            .connect_lazy(),
    );
    let (grpc_stop, grpc_signal) = oneshot::channel();
    let grpc_pool = runtime_pool.clone();
    let grpc_task = tokio::spawn(platform_runtime::module_config::serve(
        grpc_listener,
        grpc_pool,
        redis_manager.clone(),
        redis_client,
        module_identity_client,
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
    let mut authorization_client = identity_client.clone();
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

    let operator_login = client
        .post(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, &origin)
        .header("x-correlation-id", "corr_operator_login")
        .json(&serde_json::json!({
            "email": operator_email,
            "password": operator_password
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
    let duplicate_slug_identification = format!(
        "8{:018}",
        Uuid::new_v4().as_u128() % 1_000_000_000_000_000_000
    );
    let conflicting_slug = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, &origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", &csrf_cookie)
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
    let duplicate_slug_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_registry.tenants WHERE slug=$1",
    )
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
            namespace: failed_namespace,
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
        "outbox events are acknowledged durably"
    );
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
            "scopes": ["payment_intents.create"]
        }))
        .send()
        .await
        .expect("tenant owner can create an API key through BFF and gRPC");
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
    let revoked_key_status: String = sqlx::query_scalar(
        "SELECT status FROM identity_access.api_keys WHERE tenant_id=$1 AND id=$2",
    )
    .bind(created_tenant_id)
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("revocation persists in PostgreSQL");
    assert_eq!(revoked_key_status, "revoked");
    let api_key_audit: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE resource_id=$1 AND action='api_key.created' AND outcome='completed') AND EXISTS (SELECT 1 FROM identity_access.audit_events WHERE resource_id=$1 AND action='api_key.revoked' AND outcome='completed')",
    )
    .bind(api_key_id)
    .fetch_one(&migration_pool)
    .await
    .expect("API key creation and revocation are audited");
    assert!(api_key_audit);

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

fn cookie_value(cookies: &[String], name: &str) -> String {
    cookies
        .iter()
        .find_map(|cookie| {
            cookie
                .strip_prefix(&format!("{name}="))
                .and_then(|cookie| cookie.split(';').next())
                .map(str::to_owned)
        })
        .expect("required session cookie is present")
}
