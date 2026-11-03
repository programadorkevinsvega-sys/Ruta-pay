use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::{IdentityError, IdentityService};

impl IdentityService {
    pub async fn authorize_operator_operation(
        &self,
        token: &str,
        csrf_token: &str,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityError> {
        if permission.is_empty() || permission.len() > 120 {
            return Err(IdentityError::AuthorizationDenied);
        }
        let token_digest = Sha256::digest(token.as_bytes());
        let csrf_digest = Sha256::digest(csrf_token.as_bytes());
        let mut transaction = self.pool.begin().await?;
        let operator = sqlx::query(
            "SELECT s.subject_id FROM identity_access.sessions s \
             JOIN identity_access.operator_users u ON u.id=s.subject_id AND u.status='active' \
             WHERE s.token_digest=$1 AND s.csrf_digest=$2 AND s.subject_kind='operator_user' \
             AND s.revoked_at IS NULL AND s.expires_at>now()",
        )
        .bind(token_digest.as_slice())
        .bind(csrf_digest.as_slice())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::AuthenticationFailed)?;
        let operator_id: Uuid = operator
            .try_get("subject_id")
            .map_err(IdentityError::Database)?;
        let permitted = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (\
                SELECT 1 FROM identity_access.operator_users u \
                JOIN identity_access.operator_role_permissions p ON p.role=u.role \
                WHERE u.id=$1 AND p.permission=$2 AND u.status='active'\
             )",
        )
        .bind(operator_id)
        .bind(permission)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.audit_events \
             (event_id, actor_kind, actor_id, action, outcome, correlation_id, trace_id, safe_details) \
             VALUES ($1, 'operator_user', $2, 'operator.authorization.checked', $3, $4, $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(operator_id)
        .bind(if permitted { "allowed" } else { "denied" })
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .bind(serde_json::json!({ "permission": permission }))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        if permitted {
            Ok(operator_id)
        } else {
            Err(IdentityError::AuthorizationDenied)
        }
    }

    pub async fn authorize_tenant_operation(
        &self,
        token: &str,
        csrf_token: &str,
        tenant_id: Uuid,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityError> {
        if permission.is_empty() || permission.len() > 120 {
            return Err(IdentityError::AuthorizationDenied);
        }
        let token_digest = Sha256::digest(token.as_bytes());
        let csrf_digest = Sha256::digest(csrf_token.as_bytes());
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant_id.to_string())
            .execute(&mut *transaction)
            .await?;
        let actor = sqlx::query(
            "SELECT s.subject_id FROM identity_access.sessions s \
             JOIN identity_access.tenant_users u ON u.id=s.subject_id AND u.status='active' \
             WHERE s.token_digest=$1 AND s.csrf_digest=$2 AND s.subject_kind='tenant_user' \
             AND s.revoked_at IS NULL AND s.expires_at>now()",
        )
        .bind(token_digest.as_slice())
        .bind(csrf_digest.as_slice())
        .fetch_optional(&mut *transaction)
        .await?;
        let actor_id = actor
            .map(|row| row.try_get("subject_id").map_err(IdentityError::Database))
            .transpose()?
            .ok_or(IdentityError::AuthenticationFailed)?;
        let permitted = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (\
                SELECT 1 FROM identity_access.tenant_memberships m \
                JOIN identity_access.tenant_role_permissions p ON p.role=m.role \
                WHERE m.user_id=$1 AND m.tenant_id=$2 AND m.status='active' AND p.permission=$3\
             )",
        )
        .bind(actor_id)
        .bind(tenant_id)
        .bind(permission)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.audit_events \
             (event_id, actor_kind, actor_id, tenant_id, action, resource_id, outcome, correlation_id, trace_id, safe_details) \
             VALUES ($1, 'tenant_user', $2, $3, 'tenant.authorization.checked', $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(actor_id)
        .bind(tenant_id)
        .bind(if permitted { "allowed" } else { "denied" })
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .bind(serde_json::json!({ "permission": permission }))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        if permitted {
            Ok(actor_id)
        } else {
            Err(IdentityError::AuthorizationDenied)
        }
    }
}
