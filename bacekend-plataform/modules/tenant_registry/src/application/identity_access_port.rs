use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum IdentityAccessError {
    #[error("authentication failed")]
    Authentication,
    #[error("authorization denied")]
    Authorization,
    #[error("identity dependency unavailable")]
    Unavailable,
    #[error("identity request invalid")]
    Invalid,
}

#[async_trait]
pub trait IdentityAccessPort: Send + Sync {
    async fn validate_tenant_session(
        &self,
        token: &str,
        csrf_token: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError>;

    async fn authorize_tenant(
        &self,
        token: &str,
        csrf_token: &str,
        tenant_id: Uuid,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError>;

    async fn authorize_operator(
        &self,
        token: &str,
        csrf_token: &str,
        permission: &str,
        correlation_id: &str,
    ) -> Result<Uuid, IdentityAccessError>;
}
