use std::{fmt, time::Duration};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use redis::{AsyncCommands, aio::ConnectionManager};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{
    api_key::{ApiKeyEnvironment, NewApiKey, create_api_key},
    email_address::{EmailAddress, EmailAddressError},
    password::{PasswordError, validate_password},
    password_hash::{PasswordHashError, hash_password, verify_password},
};

const SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
const LOGIN_WINDOW_SECONDS: u64 = 60;
const LOGIN_WINDOW_LIMIT: u64 = 10;
const REGISTRATION_WINDOW_SECONDS: u64 = 300;
const REGISTRATION_ACCOUNT_LIMIT: i64 = 5;
const REGISTRATION_IP_LIMIT: i64 = 20;

#[derive(Clone)]
pub struct IdentityService {
    pool: PgPool,
    redis: ConnectionManager,
}

impl IdentityService {
    pub fn new(pool: PgPool, redis: ConnectionManager) -> Self {
        Self { pool, redis }
    }

    pub async fn register_tenant_user(
        &self,
        email: &str,
        password: &str,
        source_ip: &str,
        correlation_id: &str,
        traceparent: Option<&str>,
    ) -> Result<Uuid, IdentityError> {
        let email = EmailAddress::parse(email)?;
        self.enforce_registration_rate_limit(email.as_str(), source_ip)
            .await?;
        validate_password(password)?;
        let password_owned = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || hash_password(&password_owned))
            .await
            .map_err(|_| IdentityError::HashingTaskFailed)??;
        let user_id = Uuid::new_v4();
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "INSERT INTO identity_access.tenant_users (id, email_normalized, password_hash, status) VALUES ($1, $2, $3, 'active')",
        )
        .bind(user_id)
        .bind(email.as_str())
        .bind(password_hash)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = result {
            if error
                .as_database_error()
                .and_then(|db| db.code())
                .as_deref()
                == Some("23505")
            {
                return Err(IdentityError::AccountConflict);
            }
            return Err(IdentityError::Database(error));
        }
        let event_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO identity_access.outbox_events (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload, traceparent, correlation_id) VALUES ($1, $2, 1, 'identity.tenant_user.registered', 1, 'platform_operator', $3, $4, $5)",
        )
        .bind(event_id)
        .bind(user_id)
        .bind(serde_json::json!({ "user_id": user_id, "status": "active" }))
        .bind(traceparent)
        .bind(correlation_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.audit_events (event_id, actor_kind, actor_id, action, resource_id, outcome, correlation_id, trace_id) VALUES ($1, 'tenant_user', $2, 'tenant_user.registered', $2, 'completed', $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(user_id)
    }

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

    pub async fn revoke_operator_session(
        &self,
        token: &str,
        correlation_id: &str,
        traceparent: Option<&str>,
    ) -> Result<(), IdentityError> {
        self.revoke_session_for_kind(token, "operator_user", correlation_id, traceparent)
            .await
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

    async fn enforce_registration_rate_limit(
        &self,
        email: &str,
        source_ip: &str,
    ) -> Result<(), IdentityError> {
        let source_ip = source_ip
            .parse::<std::net::IpAddr>()
            .map_err(|_| IdentityError::SourceAddressInvalid)?;
        let account_digest = Sha256::digest(email.as_bytes());
        let ip_digest = Sha256::digest(source_ip.to_string().as_bytes());
        let account_key = format!(
            "platform:identity:registration:account:{}",
            URL_SAFE_NO_PAD.encode(account_digest)
        );
        let ip_key = format!(
            "platform:identity:registration:ip:{}",
            URL_SAFE_NO_PAD.encode(ip_digest)
        );

        // Increment both counters and set their expirations atomically so concurrent
        // registration requests cannot bypass the fixed-window limit.
        const RATE_LIMIT_SCRIPT: &str = r#"
            local account_count = redis.call('INCR', KEYS[1])
            if account_count == 1 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            local ip_count = redis.call('INCR', KEYS[2])
            if ip_count == 1 then
                redis.call('EXPIRE', KEYS[2], ARGV[1])
            end
            return {account_count, ip_count}
        "#;
        let mut connection = self.redis.clone();
        let (account_count, ip_count): (i64, i64) = redis::cmd("EVAL")
            .arg(RATE_LIMIT_SCRIPT)
            .arg(2)
            .arg(&account_key)
            .arg(&ip_key)
            .arg(REGISTRATION_WINDOW_SECONDS)
            .query_async(&mut connection)
            .await
            .map_err(IdentityError::Redis)?;

        if account_count > REGISTRATION_ACCOUNT_LIMIT || ip_count > REGISTRATION_IP_LIMIT {
            return Err(IdentityError::RateLimited);
        }
        Ok(())
    }

    async fn enforce_login_rate_limit(
        &self,
        email: &str,
        source_ip: &str,
    ) -> Result<(), IdentityError> {
        let source_ip = source_ip
            .parse::<std::net::IpAddr>()
            .map_err(|_| IdentityError::SourceAddressInvalid)?;
        let account_digest = Sha256::digest(email.as_bytes());
        let ip_digest = Sha256::digest(source_ip.to_string().as_bytes());
        let account_key = format!(
            "platform:identity:login:account:{}",
            URL_SAFE_NO_PAD.encode(account_digest)
        );
        let ip_key = format!(
            "platform:identity:login:ip:{}",
            URL_SAFE_NO_PAD.encode(ip_digest)
        );
        let mut connection = self.redis.clone();
        let account_count: u64 = redis::cmd("INCR")
            .arg(&account_key)
            .query_async(&mut connection)
            .await
            .map_err(IdentityError::Redis)?;
        let mut connection = self.redis.clone();
        let ip_count: u64 = redis::cmd("INCR")
            .arg(&ip_key)
            .query_async(&mut connection)
            .await
            .map_err(IdentityError::Redis)?;
        if account_count == 1 {
            let mut connection = self.redis.clone();
            let _: bool = connection
                .expire(&account_key, LOGIN_WINDOW_SECONDS as i64)
                .await
                .map_err(IdentityError::Redis)?;
        }
        if ip_count == 1 {
            let mut connection = self.redis.clone();
            let _: bool = connection
                .expire(&ip_key, LOGIN_WINDOW_SECONDS as i64)
                .await
                .map_err(IdentityError::Redis)?;
        }
        if account_count > LOGIN_WINDOW_LIMIT || ip_count > LOGIN_WINDOW_LIMIT * 5 {
            return Err(IdentityError::RateLimited);
        }
        Ok(())
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

    pub async fn create_api_key(
        &self,
        session_token: &str,
        tenant_id: Uuid,
        name: &str,
        environment: &str,
        scopes: &[String],
        correlation_id: &str,
    ) -> Result<CreatedApiKey, IdentityError> {
        let actor_id = self
            .authorized_tenant_actor(session_token, tenant_id)
            .await?;
        let environment = match environment {
            "test" => ApiKeyEnvironment::Test,
            "live" => ApiKeyEnvironment::Live,
            _ => return Err(IdentityError::ApiKeyInputInvalid),
        };
        let key = create_api_key(name, environment, scopes)
            .map_err(|_| IdentityError::ApiKeyInputInvalid)?;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id, actor_id).await?;
        sqlx::query(
            "INSERT INTO identity_access.api_keys (id, tenant_id, name, environment, prefix, secret_digest, scopes, status, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,'active',$8)",
        )
        .bind(key.id)
        .bind(tenant_id)
        .bind(&key.name)
        .bind(key.environment.as_str())
        .bind(&key.prefix)
        .bind(key.digest.as_slice())
        .bind(&key.scopes)
        .bind(actor_id)
        .execute(&mut *transaction)
        .await?;
        let payload = serde_json::json!({"tenant_id": tenant_id, "api_key_id": key.id, "status": "active", "environment": key.environment.as_str()});
        sqlx::query(
            "INSERT INTO identity_access.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,1,'api_key.status.changed',1,'tenant',$3,$4)",
        )
        .bind(Uuid::new_v4())
        .bind(key.id)
        .bind(payload)
        .bind(correlation_id)
        .execute(&mut *transaction)
        .await?;
        insert_audit(
            &mut transaction,
            "api_key.created",
            actor_id,
            tenant_id,
            key.id,
            correlation_id,
        )
        .await?;
        transaction.commit().await?;
        Ok(CreatedApiKey { tenant_id, key })
    }

    pub async fn list_api_keys(
        &self,
        session_token: &str,
        tenant_id: Uuid,
    ) -> Result<Vec<ApiKeyMetadata>, IdentityError> {
        let actor_id = self
            .authorized_tenant_actor(session_token, tenant_id)
            .await?;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id, actor_id).await?;
        let rows = sqlx::query(
            "SELECT id,name,prefix,environment,scopes,status,created_at,revoked_at FROM identity_access.api_keys WHERE tenant_id=$1 ORDER BY created_at DESC",
        )
        .bind(tenant_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            keys.push(ApiKeyMetadata {
                id: row.try_get("id").map_err(IdentityError::Database)?,
                name: row.try_get("name").map_err(IdentityError::Database)?,
                prefix: row.try_get("prefix").map_err(IdentityError::Database)?,
                environment: row
                    .try_get("environment")
                    .map_err(IdentityError::Database)?,
                scopes: row.try_get("scopes").map_err(IdentityError::Database)?,
                status: row.try_get("status").map_err(IdentityError::Database)?,
                created_at: row.try_get("created_at").map_err(IdentityError::Database)?,
                revoked_at: row.try_get("revoked_at").map_err(IdentityError::Database)?,
            });
        }
        transaction.commit().await?;
        Ok(keys)
    }

    pub async fn revoke_api_key(
        &self,
        session_token: &str,
        tenant_id: Uuid,
        key_id: Uuid,
        correlation_id: &str,
    ) -> Result<(), IdentityError> {
        let actor_id = self
            .authorized_tenant_actor(session_token, tenant_id)
            .await?;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id, actor_id).await?;
        let result = sqlx::query("UPDATE identity_access.api_keys SET status='revoked',revoked_at=now() WHERE id=$1 AND tenant_id=$2 AND status='active'")
            .bind(key_id).bind(tenant_id).execute(&mut *transaction).await?;
        if result.rows_affected() == 0 {
            return Err(IdentityError::ApiKeyNotFound);
        }
        let payload =
            serde_json::json!({"tenant_id": tenant_id, "api_key_id": key_id, "status": "revoked"});
        sqlx::query("INSERT INTO identity_access.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,2,'api_key.status.changed',1,'tenant',$3,$4)")
            .bind(Uuid::new_v4()).bind(key_id).bind(payload).bind(correlation_id).execute(&mut *transaction).await?;
        insert_audit(
            &mut transaction,
            "api_key.revoked",
            actor_id,
            tenant_id,
            key_id,
            correlation_id,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn authorized_tenant_actor(
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
        let row = sqlx::query(
            "SELECT s.subject_id FROM identity_access.sessions s JOIN identity_access.tenant_memberships m ON m.user_id=s.subject_id AND m.tenant_id=$2 AND m.status='active' JOIN identity_access.tenant_role_permissions p ON p.role=m.role AND p.permission='api_keys.manage' WHERE s.token_digest=$1 AND s.subject_kind='tenant_user' AND s.revoked_at IS NULL AND s.expires_at>now()",
        )
        .bind(token_digest.as_slice()).bind(tenant_id).fetch_optional(&mut *transaction).await?;
        let actor = row
            .map(|row| row.try_get("subject_id").map_err(IdentityError::Database))
            .transpose()?
            .ok_or(IdentityError::AuthorizationDenied);
        transaction.commit().await?;
        actor
    }
}

pub struct TenantMembershipRecord {
    pub tenant_id: Uuid,
    pub role: String,
    pub membership_status: String,
}

async fn set_tenant_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    actor_id: Uuid,
) -> Result<(), IdentityError> {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true),set_config('app.actor_id',$2,true)")
        .bind(tenant_id.to_string())
        .bind(actor_id.to_string())
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn insert_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    action: &str,
    actor_id: Uuid,
    tenant_id: Uuid,
    resource_id: Uuid,
    correlation_id: &str,
) -> Result<(), IdentityError> {
    sqlx::query("INSERT INTO identity_access.audit_events (event_id,actor_kind,actor_id,tenant_id,action,resource_id,outcome,correlation_id) VALUES ($1,'tenant_user',$2,$3,$4,$5,'completed',$6)")
        .bind(Uuid::new_v4()).bind(actor_id).bind(tenant_id).bind(action).bind(resource_id).bind(correlation_id).execute(&mut **transaction).await?;
    Ok(())
}

pub struct CreatedApiKey {
    pub tenant_id: Uuid,
    pub key: NewApiKey,
}

pub struct ApiKeyMetadata {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub environment: String,
    pub scopes: Vec<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

pub struct CreatedSession {
    pub user_id: Uuid,
    pub session_token: SecretToken,
    pub csrf_token: SecretToken,
    pub expires_at: DateTime<Utc>,
}

pub struct SecretToken(String);

impl SecretToken {
    pub fn expose_once(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error(transparent)]
    Email(#[from] EmailAddressError),
    #[error(transparent)]
    Password(#[from] PasswordError),
    #[error(transparent)]
    PasswordHash(#[from] PasswordHashError),
    #[error("account already exists")]
    AccountConflict,
    #[error("operator bootstrap has already been completed")]
    OperatorBootstrapClosed,
    #[error("authentication failed")]
    AuthenticationFailed,
    #[error("authorization denied")]
    AuthorizationDenied,
    #[error("API key input is invalid")]
    ApiKeyInputInvalid,
    #[error("API key not found")]
    ApiKeyNotFound,
    #[error("password hashing task failed")]
    HashingTaskFailed,
    #[error("operating system random source is unavailable")]
    RandomSourceUnavailable,
    #[error("session expiration configuration is invalid")]
    TimeConfigurationInvalid,
    #[error("login source address is invalid")]
    SourceAddressInvalid,
    #[error("login rate limit exceeded")]
    RateLimited,
    #[error("rate limiting dependency is unavailable")]
    Redis(#[source] redis::RedisError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
