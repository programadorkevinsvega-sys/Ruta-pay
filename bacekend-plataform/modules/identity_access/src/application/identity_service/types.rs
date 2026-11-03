use std::fmt;

use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{
    api_key::NewApiKey, email_address::EmailAddressError, password::PasswordError,
    password_hash::PasswordHashError,
};

pub struct TenantMembershipRecord {
    pub tenant_id: Uuid,
    pub role: String,
    pub membership_status: String,
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

pub struct ApiKeyAuthorization {
    pub key_id: Uuid,
    pub tenant_id: Uuid,
    pub environment: String,
}

pub struct CreatedSession {
    pub user_id: Uuid,
    pub session_token: SecretToken,
    pub csrf_token: SecretToken,
    pub expires_at: DateTime<Utc>,
}

pub struct SecretToken(pub(super) String);

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
