use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::{IdentityError, IdentityService, TenantMembershipRecord};

impl IdentityService {
    pub async fn validate_operator_session(
        &self,
        token: &str,
        csrf_token: &str,
    ) -> Result<Uuid, IdentityError> {
        let token_digest = Sha256::digest(token.as_bytes());
        let csrf_digest = Sha256::digest(csrf_token.as_bytes());
        let row = sqlx::query(
            "SELECT s.subject_id FROM identity_access.sessions s JOIN identity_access.operator_users u ON u.id=s.subject_id AND u.status='active' WHERE s.token_digest=$1 AND s.csrf_digest=$2 AND s.subject_kind='operator_user' AND s.revoked_at IS NULL AND s.expires_at>now()",
        )
        .bind(token_digest.as_slice())
        .bind(csrf_digest.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| row.try_get("subject_id").map_err(IdentityError::Database))
            .transpose()?
            .ok_or(IdentityError::AuthenticationFailed)
    }

    pub async fn validate_tenant_session(
        &self,
        token: &str,
        csrf_token: &str,
    ) -> Result<Uuid, IdentityError> {
        let token_digest = Sha256::digest(token.as_bytes());
        let csrf_digest = Sha256::digest(csrf_token.as_bytes());
        let row = sqlx::query(
            "SELECT s.subject_id FROM identity_access.sessions s JOIN identity_access.tenant_users u ON u.id=s.subject_id AND u.status='active' WHERE s.token_digest=$1 AND s.csrf_digest=$2 AND s.subject_kind='tenant_user' AND s.revoked_at IS NULL AND s.expires_at>now()",
        )
        .bind(token_digest.as_slice())
        .bind(csrf_digest.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| row.try_get("subject_id").map_err(IdentityError::Database))
            .transpose()?
            .ok_or(IdentityError::AuthenticationFailed)
    }

    pub async fn list_tenant_memberships(
        &self,
        token: &str,
        csrf_token: &str,
    ) -> Result<(Uuid, Vec<TenantMembershipRecord>), IdentityError> {
        let actor_id = self.validate_tenant_session(token, csrf_token).await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.actor_id',$1,true)")
            .bind(actor_id.to_string())
            .execute(&mut *transaction)
            .await?;
        let rows = sqlx::query(
            "SELECT tenant_id, role, status AS membership_status \
             FROM identity_access.tenant_memberships \
             WHERE user_id=$1 AND status='active' ORDER BY tenant_id",
        )
        .bind(actor_id)
        .fetch_all(&mut *transaction)
        .await?;
        let memberships = rows
            .into_iter()
            .map(|row| {
                Ok(TenantMembershipRecord {
                    tenant_id: row.try_get("tenant_id")?,
                    role: row.try_get("role")?,
                    membership_status: row.try_get("membership_status")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        transaction.commit().await?;
        Ok((actor_id, memberships))
    }
}
