use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    application::identity_service::IdentityError,
    domain::{
        email_address::EmailAddress, password::validate_password, password_hash::hash_password,
    },
};

#[derive(Clone)]
pub struct OperatorBootstrapService {
    pool: PgPool,
}

impl OperatorBootstrapService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn bootstrap_first_operator(
        &self,
        email: &str,
        password: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityError> {
        let email = EmailAddress::parse(email)?;
        validate_password(password)?;
        let password_owned = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || hash_password(&password_owned))
            .await
            .map_err(|_| IdentityError::HashingTaskFailed)??;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("LOCK TABLE identity_access.operator_users IN EXCLUSIVE MODE")
            .execute(&mut *transaction)
            .await?;
        let operator_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM identity_access.operator_users)")
                .fetch_one(&mut *transaction)
                .await?;
        if operator_exists {
            return Err(IdentityError::OperatorBootstrapClosed);
        }
        let operator_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO identity_access.operator_users (id, email_normalized, password_hash, status, role) VALUES ($1, $2, $3, 'active', 'platform_owner')",
        )
        .bind(operator_id)
        .bind(email.as_str())
        .bind(password_hash)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.outbox_events (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload) VALUES ($1, $2, 1, 'identity.operator.bootstrap.completed', 1, 'platform_operator', $3)",
        )
        .bind(Uuid::new_v4())
        .bind(operator_id)
        .bind(serde_json::json!({ "operator_id": operator_id, "role": "platform_owner" }))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.audit_events (event_id, actor_kind, actor_id, action, resource_id, outcome, correlation_id, trace_id) VALUES ($1, 'system', NULL, 'operator.bootstrap.completed', $2, 'completed', $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(operator_id)
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(operator_id)
    }
}
