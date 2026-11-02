use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::domain::{email_address::EmailAddress, password_hash::verify_password};
use super::{CreatedSession, IdentityError, IdentityService, SecretToken};

const SESSION_LIFETIME: std::time::Duration = std::time::Duration::from_secs(12 * 60 * 60);

impl IdentityService {
    pub async fn create_operator_session(
        &self,
        email: &str,
        password: &str,
        source_ip: &str,
    ) -> Result<CreatedSession, IdentityError> {
        let email = EmailAddress::parse(email).map_err(|_| IdentityError::AuthenticationFailed)?;
        self.enforce_login_rate_limit(email.as_str(), source_ip)
            .await?;
        let row = sqlx::query(
            "SELECT id, password_hash, status FROM identity_access.operator_users WHERE email_normalized = $1",
        )
        .bind(email.as_str())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::AuthenticationFailed)?;
        let user_id: Uuid = row.try_get("id").map_err(IdentityError::Database)?;
        let password_hash: String = row
            .try_get("password_hash")
            .map_err(IdentityError::Database)?;
        let status: String = row.try_get("status").map_err(IdentityError::Database)?;
        let password_owned = password.to_owned();
        let verified =
            tokio::task::spawn_blocking(move || verify_password(&password_owned, &password_hash))
                .await
                .map_err(|_| IdentityError::HashingTaskFailed)?;
        if status != "active" || !verified {
            return Err(IdentityError::AuthenticationFailed);
        }
        self.issue_session(user_id, "operator_user").await
    }

    pub async fn create_tenant_session(
        &self,
        email: &str,
        password: &str,
        source_ip: &str,
    ) -> Result<CreatedSession, IdentityError> {
        let email = EmailAddress::parse(email).map_err(|_| IdentityError::AuthenticationFailed)?;
        self.enforce_login_rate_limit(email.as_str(), source_ip)
            .await?;
        let row = sqlx::query(
            "SELECT id, password_hash, status FROM identity_access.tenant_users WHERE email_normalized = $1",
        )
        .bind(email.as_str())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::AuthenticationFailed)?;
        let user_id: Uuid = row.try_get("id").map_err(IdentityError::Database)?;
        let password_hash: String = row
            .try_get("password_hash")
            .map_err(IdentityError::Database)?;
        let status: String = row.try_get("status").map_err(IdentityError::Database)?;
        let password_owned = password.to_owned();
        let verified =
            tokio::task::spawn_blocking(move || verify_password(&password_owned, &password_hash))
                .await
                .map_err(|_| IdentityError::HashingTaskFailed)?;
        if status != "active" || !verified {
            return Err(IdentityError::AuthenticationFailed);
        }

        let mut random_bytes = [0_u8; 32];
        let mut csrf_bytes = [0_u8; 32];
        getrandom::fill(&mut random_bytes).map_err(|_| IdentityError::RandomSourceUnavailable)?;
        getrandom::fill(&mut csrf_bytes).map_err(|_| IdentityError::RandomSourceUnavailable)?;
        let token = URL_SAFE_NO_PAD.encode(random_bytes);
        let csrf_token = URL_SAFE_NO_PAD.encode(csrf_bytes);
        let token_digest = Sha256::digest(token.as_bytes());
        let csrf_digest = Sha256::digest(csrf_token.as_bytes());
        let session_id = Uuid::new_v4();
        let expires_at = Utc::now()
            + chrono::TimeDelta::from_std(SESSION_LIFETIME)
                .map_err(|_| IdentityError::TimeConfigurationInvalid)?;
        sqlx::query(
            "INSERT INTO identity_access.sessions (id, subject_kind, subject_id, token_digest, csrf_digest, expires_at) VALUES ($1, 'tenant_user', $2, $3, $4, $5)",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(token_digest.as_slice())
        .bind(csrf_digest.as_slice())
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(CreatedSession {
            user_id,
            session_token: SecretToken(token),
            csrf_token: SecretToken(csrf_token),
            expires_at,
        })
    }

    async fn issue_session(
        &self,
        user_id: Uuid,
        subject_kind: &'static str,
    ) -> Result<CreatedSession, IdentityError> {
        let mut random_bytes = [0_u8; 32];
        let mut csrf_bytes = [0_u8; 32];
        getrandom::fill(&mut random_bytes).map_err(|_| IdentityError::RandomSourceUnavailable)?;
        getrandom::fill(&mut csrf_bytes).map_err(|_| IdentityError::RandomSourceUnavailable)?;
        let token = URL_SAFE_NO_PAD.encode(random_bytes);
        let csrf_token = URL_SAFE_NO_PAD.encode(csrf_bytes);
        let token_digest = Sha256::digest(token.as_bytes());
        let csrf_digest = Sha256::digest(csrf_token.as_bytes());
        let session_id = Uuid::new_v4();
        let expires_at = Utc::now()
            + chrono::TimeDelta::from_std(SESSION_LIFETIME)
                .map_err(|_| IdentityError::TimeConfigurationInvalid)?;
        sqlx::query(
            "INSERT INTO identity_access.sessions (id, subject_kind, subject_id, token_digest, csrf_digest, expires_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session_id)
        .bind(subject_kind)
        .bind(user_id)
        .bind(token_digest.as_slice())
        .bind(csrf_digest.as_slice())
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(CreatedSession {
            user_id,
            session_token: SecretToken(token),
            csrf_token: SecretToken(csrf_token),
            expires_at,
        })
    }
}
