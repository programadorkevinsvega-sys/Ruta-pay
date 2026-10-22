use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use platform_contracts::{
    identity_access::{
        AuthorizeOperatorOperationRequest, identity_access_client::IdentityAccessClient,
    },
    runtime::runtime_probe_client::RuntimeProbeClient,
};
use platform_identity_access::application::operator_bootstrap::OperatorBootstrapService;
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

impl Visit for FieldCapture<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

#[tokio::test]
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
        "GRANT USAGE ON SCHEMA identity_access, tenant_registry TO \"{runtime_role}\""
    )))
    .execute(&migration_pool)
    .await
    .expect("runtime role receives schema usage");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA identity_access, tenant_registry TO \"{runtime_role}\"")))
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
    let redis_manager = redis::aio::ConnectionManager::new(redis_client)
        .await
        .expect("real development Redis accepts a connection");
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.expect("gRPC bind");
    let grpc_addr = grpc_listener.local_addr().expect("gRPC address");
    let (grpc_stop, grpc_signal) = oneshot::channel();
    let grpc_pool = runtime_pool.clone();
    let grpc_task = tokio::spawn(platform_runtime::module_config::serve(
        grpc_listener,
        grpc_pool,
        redis_manager.clone(),
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
    let mut authorization_client = identity_client.clone();
    let http_listener = TcpListener::bind("127.0.0.1:0").await.expect("BFF bind");
    let http_addr = http_listener.local_addr().expect("BFF address");
    let origin = format!("http://localhost:{}", http_addr.port());
    let (http_stop, http_signal) = oneshot::channel();
    let http_task = tokio::spawn(async move {
        axum::serve(
            http_listener,
            payvia_platform_bff::http::routes::router(runtime_client, identity_client)
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
        .default_headers(default_headers)
        .build()
        .expect("HTTP acceptance client builds");
    let email = format!("acceptance_{}@example.test", Uuid::new_v4().simple());
    let password = format!("A-valid-password-{}!", Uuid::new_v4().simple());
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
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
    )
        .bind(&database_name)
        .execute(&admin_pool)
        .await
        .expect("temporary database sessions terminate");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE \"{database_name}\""
    )))
    .execute(&admin_pool)
    .await
    .expect("temporary acceptance database is dropped");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP ROLE \"{runtime_role}\"")))
        .execute(&admin_pool)
        .await
        .expect("temporary runtime role is dropped");
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
