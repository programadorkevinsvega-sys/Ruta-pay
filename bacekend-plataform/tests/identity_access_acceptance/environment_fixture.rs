use super::{
    database_fixture, duplicate_registration, operator_redis_fixture,
    outbox_transaction_rollback, server_fixture, credential_validation, concurrent_registration,
};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use reqwest::{StatusCode, header};
use tracing_subscriber::prelude::*;
use uuid::Uuid;
use super::identity_access_acceptance_support::{AcceptanceDatabaseCleanup, EventCapture};
use sqlx::PgPool;

pub struct IdentityEnvironment {
    pub provider: SdkTracerProvider,
    pub captured_events:
        std::sync::Arc<std::sync::Mutex<Vec<std::collections::HashMap<String, String>>>>,
    pub admin_pool: PgPool,
    pub migration_pool: PgPool,
    pub runtime_pool: PgPool,
    pub runtime_role: String,
    pub database_cleanup: AcceptanceDatabaseCleanup,
    pub redis_manager: redis::aio::ConnectionManager,
    pub operator_email: String,
    pub operator_password: String,
    pub operator_id: Uuid,
    pub servers: server_fixture::ServerFixture,
    pub email: String,
    pub password: String,
    pub registered_user_id: Uuid,
}

pub async fn setup() -> IdentityEnvironment {
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

    let database = database_fixture::create().await;
    let admin_pool = database.admin_pool;
    let migration_pool = database.migration_pool;
    let runtime_pool = database.runtime_pool;
    let runtime_role = database.runtime_role;
    let redis_url = database.redis_url;
    let database_cleanup = database.cleanup;

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

    let servers =
        server_fixture::ServerFixture::start(&runtime_pool, &redis_manager, redis_client).await;
    let http_addr = servers.http_addr;
    let origin = servers.origin.clone();
    let client = servers.client.clone();

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
    IdentityEnvironment {
        provider,
        captured_events,
        admin_pool,
        migration_pool,
        runtime_pool,
        runtime_role,
        database_cleanup,
        redis_manager,
        operator_email,
        operator_password,
        operator_id,
        servers,
        email,
        password,
        registered_user_id,
    }
}
