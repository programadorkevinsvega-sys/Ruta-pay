use platform_identity_access::application::operator_bootstrap::OperatorBootstrapService;
use sqlx::PgPool;
use uuid::Uuid;

pub(super) struct OperatorState {
    pub(super) email: String,
    pub(super) password: String,
    pub(super) id: Uuid,
}

pub(super) async fn bootstrap(runtime_pool: &PgPool) -> OperatorState {
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

    OperatorState {
        email: operator_email,
        password: operator_password,
        id: operator_id,
    }
}
