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

struct RedisAclUserCleanup {
    redis_manager: redis::aio::ConnectionManager,
    username: String,
    armed: bool,
}

impl RedisAclUserCleanup {
    async fn remove(&mut self) {
        Self::delete_user(&self.redis_manager, &self.username).await;
        self.armed = false;
    }

    async fn delete_user(
        redis_manager: &redis::aio::ConnectionManager,
        username: &str,
    ) {
        let mut connection = redis_manager.clone();
        let removed: i64 = redis::cmd("ACL")
            .arg("DELUSER")
            .arg(username)
            .query_async(&mut connection)
            .await
            .expect("temporary restricted Redis user is removed");
        assert_eq!(removed, 1, "temporary restricted Redis user existed");
    }
}

impl Drop for RedisAclUserCleanup {
    fn drop(&mut self) {
        if self.armed {
            let redis_manager = self.redis_manager.clone();
            let username = self.username.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                runtime.block_on(async move {
                    Self::delete_user(&redis_manager, &username).await;
                })
            });
        }
    }
}

pub async fn bootstrap_and_verify_redis_failure(
    runtime_pool: &PgPool,
    migration_pool: &PgPool,
    redis_url: &str,
    redis_acl_admin_url: &str,
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

    let redis_client = redis::Client::open(redis_url).expect("Redis URL parses");
    let redis_manager = redis::aio::ConnectionManager::new(redis_client.clone())
        .await
        .expect("real development Redis accepts a connection");

    let redis_acl_admin_client =
        redis::Client::open(redis_acl_admin_url).expect("Redis ACL administrator URL parses");
    let redis_acl_admin_manager =
        redis::aio::ConnectionManager::new(redis_acl_admin_client)
            .await
            .expect("dedicated Redis ACL administrator connects");
    let restricted_redis_user = format!("phase2_limited_{}", Uuid::new_v4().simple());
    let restricted_redis_password = Uuid::new_v4().simple().to_string();
    let mut redis_acl_admin = redis_acl_admin_manager.clone();
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
    let mut cleanup = RedisAclUserCleanup {
        redis_manager: redis_acl_admin_manager,
        username: restricted_redis_user.clone(),
        armed: true,
    };
    let mut restricted_redis_url =
        url::Url::parse(redis_url).expect("Redis URL parses for isolated ACL user");
    restricted_redis_url
        .set_username(&restricted_redis_user)
        .expect("restricted Redis username is valid");
    restricted_redis_url
        .set_password(Some(&restricted_redis_password))
        .expect("restricted Redis password is valid");
    let restricted_redis_client = redis::Client::open(restricted_redis_url.as_str())
        .expect("restricted Redis URL parses");
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
    cleanup.remove().await;
    OperatorRedisFixture {
        operator_email,
        operator_password,
        operator_id,
        redis_client,
        redis_manager,
    }
}
