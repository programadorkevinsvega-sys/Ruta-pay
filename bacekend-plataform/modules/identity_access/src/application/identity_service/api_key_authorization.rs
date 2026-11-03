use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::{ApiKeyAuthorization, IdentityError, IdentityService};

impl IdentityService {
    pub async fn authorize_api_key_operation(
        &self,
        secret: &str,
        tenant_id: Uuid,
        environment: &str,
        required_scope: &str,
        correlation_id: &str,
    ) -> Result<ApiKeyAuthorization, IdentityError> {
        if !matches!(environment, "test" | "live")
            || required_scope.is_empty()
            || required_scope.len() > 64
            || !required_scope.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b':'
                    || byte == b'.'
                    || byte == b'_'
            })
        {
            return Err(IdentityError::ApiKeyInputInvalid);
        }
        if secret.is_empty() || secret.len() > 128 {
            return Err(IdentityError::AuthenticationFailed);
        }

        let secret_digest = Sha256::digest(secret.as_bytes());
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant_id.to_string())
            .execute(&mut *transaction)
            .await?;
        let row = sqlx::query(
            "SELECT id,environment,scopes FROM identity_access.api_keys WHERE tenant_id=$1 AND secret_digest=$2 AND environment=$3 AND status='active' AND revoked_at IS NULL FOR UPDATE",
        )
        .bind(tenant_id)
        .bind(secret_digest.as_slice())
        .bind(environment)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::AuthenticationFailed)?;

        let key_id: Uuid = row.try_get("id").map_err(IdentityError::Database)?;
        let stored_environment: String = row
            .try_get("environment")
            .map_err(IdentityError::Database)?;
        let scopes: Vec<String> = row.try_get("scopes").map_err(IdentityError::Database)?;
        let allowed =
            stored_environment == environment && scopes.iter().any(|scope| scope == required_scope);

        sqlx::query(
            "INSERT INTO identity_access.audit_events (event_id,actor_kind,actor_id,tenant_id,action,resource_id,outcome,correlation_id,trace_id,safe_details) VALUES ($1,'system',NULL,$2,'api_key.authorization.checked',$3,$4,$5,$6,$7)",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(key_id)
        .bind(if allowed { "allowed" } else { "denied" })
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .bind(serde_json::json!({
            "required_scope": required_scope,
            "environment": environment
        }))
        .execute(&mut *transaction)
        .await?;

        if !allowed {
            transaction.commit().await?;
            return Err(IdentityError::AuthorizationDenied);
        }

        sqlx::query(
            "UPDATE identity_access.api_keys SET last_used_at=now() WHERE id=$1 AND tenant_id=$2",
        )
        .bind(key_id)
        .bind(tenant_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ApiKeyAuthorization {
            key_id,
            tenant_id,
            environment: stored_environment,
        })
    }

    pub(super) async fn authorized_tenant_actor(
        &self,
        token: &str,
        tenant_id: Uuid,
    ) -> Result<Uuid, IdentityError> {
        let token_digest = Sha256::digest(token.as_bytes());
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant_id.to_string())
            .execute(&mut *transaction)
            .await?;
        let session = sqlx::query(
            "SELECT subject_id FROM identity_access.sessions WHERE token_digest=$1 AND subject_kind='tenant_user' AND revoked_at IS NULL AND expires_at>now()",
        )
        .bind(token_digest.as_slice())
        .fetch_optional(&mut *transaction)
        .await?;
        let actor_id = session
            .map(|row| row.try_get("subject_id").map_err(IdentityError::Database))
            .transpose()?;
        let has_api_key_permission = if let Some(actor_id) = actor_id {
            sqlx::query(
                "SELECT 1 FROM identity_access.tenant_memberships m JOIN identity_access.tenant_role_permissions p ON p.role=m.role WHERE m.user_id=$1 AND m.tenant_id=$2 AND m.status='active' AND p.permission='api_keys.manage'",
            )
            .bind(actor_id)
            .bind(tenant_id)
            .fetch_optional(&mut *transaction)
            .await?
            .is_some()
        } else {
            false
        };
        transaction.commit().await?;
        let actor_id = actor_id.ok_or(IdentityError::AuthenticationFailed)?;
        if !has_api_key_permission {
            return Err(IdentityError::AuthorizationDenied);
        }
        Ok(actor_id)
    }
}
