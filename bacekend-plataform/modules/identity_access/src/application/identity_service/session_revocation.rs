use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::{IdentityError, IdentityService};

impl IdentityService {
    pub async fn revoke_operator_session(
        &self,
        token: &str,
        correlation_id: &str,
        traceparent: Option<&str>,
    ) -> Result<(), IdentityError> {
        self.revoke_session_for_kind(token, "operator_user", correlation_id, traceparent)
            .await
    }

    pub async fn revoke_tenant_session(
        &self,
        token: &str,
        correlation_id: &str,
        traceparent: Option<&str>,
    ) -> Result<(), IdentityError> {
        self.revoke_session_for_kind(token, "tenant_user", correlation_id, traceparent)
            .await
    }

    async fn revoke_session_for_kind(
        &self,
        token: &str,
        subject_kind: &'static str,
        correlation_id: &str,
        traceparent: Option<&str>,
    ) -> Result<(), IdentityError> {
        let token_digest = Sha256::digest(token.as_bytes());
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE identity_access.sessions SET revoked_at = now() WHERE token_digest = $1 AND subject_kind = $2 AND revoked_at IS NULL RETURNING id, subject_id",
        )
        .bind(token_digest.as_slice())
        .bind(subject_kind)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::AuthenticationFailed)?;
        let session_id: Uuid = result.try_get("id").map_err(IdentityError::Database)?;
        let subject_id: Uuid = result
            .try_get("subject_id")
            .map_err(IdentityError::Database)?;
        let audience = if subject_kind == "operator_user" {
            "platform_operator"
        } else {
            "tenant_user"
        };
        sqlx::query(
            "INSERT INTO identity_access.outbox_events (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload, traceparent, correlation_id) VALUES ($1, $2, 1, 'identity.session.revoked', 1, $3, $4, $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(session_id)
        .bind(audience)
        .bind(serde_json::json!({ "session_id": session_id, "subject_id": subject_id, "subject_kind": subject_kind, "status": "revoked" }))
        .bind(traceparent)
        .bind(correlation_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.audit_events (event_id, actor_kind, actor_id, action, resource_id, outcome, correlation_id, trace_id) VALUES ($1, $2, $3, 'identity.session.revoked', $4, 'completed', $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(subject_kind)
        .bind(subject_id)
        .bind(session_id)
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}
