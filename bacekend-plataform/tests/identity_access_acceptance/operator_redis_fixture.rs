use platform_identity_access::application::operator_bootstrap::OperatorBootstrapService;
use sqlx::PgPool;
use uuid::Uuid;

pub struct OperatorRedisFixture {
    pub operator_email: String,
    pub operator_password: String,
    pub operator_id: Uuid,
    pub redis_client: redis::Client,
    pub redis_manager: redis::aio::ConnectionManager,
}

pub async fn bootstrap_and_verify_redis_failure(
    runtime_pool: &PgPool,
    migration_pool: &PgPool,
    redis_url: &str,
) -> OperatorRedisFixture {
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
    .fetch_one(migration_pool)
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
            .fetch_one(migration_pool)
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
            .fetch_one(migration_pool)
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
    OperatorRedisFixture {
        operator_email,
        operator_password,
        operator_id,
        redis_client,
        redis_manager,
    }
}
