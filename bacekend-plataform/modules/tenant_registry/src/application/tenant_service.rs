use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    application::identity_access_port::{IdentityAccessError, IdentityAccessPort},
    domain::tenant::{Tenant, TenantError, TenantStatus},
    infrastructure::persistence::{self, TenantRepositoryError},
};

#[derive(Clone)]
pub struct TenantService {
    pool: PgPool,
    identity: Arc<dyn IdentityAccessPort>,
}

pub struct ChangeTenantStatusCommand<'a> {
    pub session_token: &'a str,
    pub csrf_token: &'a str,
    pub tenant_id: Uuid,
    pub next: TenantStatus,
    pub reason: &'a str,
    pub expected_version: i64,
    pub correlation_id: &'a str,
}

pub struct RegisterTenantCommand<'a> {
    pub session_token: &'a str,
    pub csrf_token: &'a str,
    pub display_name: &'a str,
    pub slug: &'a str,
    pub identification_type: &'a str,
    pub identification_number: &'a str,
    pub correlation_id: &'a str,
}

impl TenantService {
    pub fn new(pool: PgPool, identity: Arc<dyn IdentityAccessPort>) -> Self {
        Self { pool, identity }
    }

    pub async fn register(
        &self,
        command: RegisterTenantCommand<'_>,
    ) -> Result<Tenant, TenantServiceError> {
        let RegisterTenantCommand {
            session_token,
            csrf_token,
            display_name,
            slug,
            identification_type,
            identification_number,
            correlation_id,
        } = command;
        let normalized_identification_number = Tenant::validate_registration(
            display_name,
            slug,
            identification_type,
            identification_number,
        )?;
        let actor_id = self
            .identity
            .validate_tenant_session(session_token, csrf_token, correlation_id)
            .await?;
        persistence::create_pending_tenant(
            &self.pool,
            persistence::CreatePendingTenantRecord {
                id: Uuid::new_v4(),
                actor_id,
                display_name,
                slug,
                identification_type,
                identification_number: &normalized_identification_number,
                correlation_id,
            },
        )
        .await
        .map_err(TenantServiceError::Database)
    }

    pub async fn list_for_operator(
        &self,
        token: &str,
        csrf_token: &str,
        query: &str,
        limit: i64,
        offset: i64,
        correlation_id: &str,
    ) -> Result<(Vec<Tenant>, i64), TenantServiceError> {
        if query.chars().count() > 120 || !(1..=100).contains(&limit) || offset < 0 {
            return Err(TenantServiceError::InvalidQuery);
        }
        let actor_id = self
            .identity
            .authorize_operator(token, csrf_token, "tenants.read", correlation_id)
            .await?;
        persistence::list_tenants(&self.pool, actor_id, query, limit, offset)
            .await
            .map_err(TenantServiceError::Database)
    }

    pub async fn get(
        &self,
        token: &str,
        csrf_token: &str,
        tenant_id: Uuid,
        correlation_id: &str,
    ) -> Result<Tenant, TenantServiceError> {
        let tenant_session = self
            .identity
            .validate_tenant_session(token, csrf_token, correlation_id)
            .await;
        let (actor_id, operator_scope) = match tenant_session {
            Ok(_) => (
                self.identity
                    .authorize_tenant(token, csrf_token, tenant_id, "tenant.read", correlation_id)
                    .await?,
                false,
            ),
            Err(IdentityAccessError::Authentication) => (
                self.identity
                    .authorize_operator(token, csrf_token, "tenants.read", correlation_id)
                    .await?,
                true,
            ),
            Err(error) => return Err(TenantServiceError::Identity(error)),
        };
        persistence::get_tenant(&self.pool, actor_id, tenant_id, operator_scope)
            .await?
            .ok_or(TenantServiceError::NotFound)
    }

    pub async fn change_status(
        &self,
        command: ChangeTenantStatusCommand<'_>,
    ) -> Result<Tenant, TenantServiceError> {
        let ChangeTenantStatusCommand {
            session_token,
            csrf_token,
            tenant_id,
            next,
            reason,
            expected_version,
            correlation_id,
        } = command;
        let required_permission = match next {
            TenantStatus::Active => "tenants.activate",
            TenantStatus::Suspended => "tenants.suspend",
            TenantStatus::Closed | TenantStatus::Closing => "tenants.close",
            TenantStatus::PendingVerification => return Err(TenantServiceError::InvalidTransition),
        };
        let actor_id = self
            .identity
            .authorize_operator(
                session_token,
                csrf_token,
                required_permission,
                correlation_id,
            )
            .await?;
        if expected_version < 1 {
            return Err(TenantServiceError::InvalidQuery);
        }
        if next == TenantStatus::Active {
            self.identity
                .authorize_operator(session_token, csrf_token, "tenants.verify", correlation_id)
                .await?;
        }
        persistence::change_tenant_status(
            &self.pool,
            persistence::ChangeTenantStatusRecord {
                tenant_id,
                actor_id,
                actor_is_operator: true,
                next,
                reason,
                correlation_id,
                expected_version,
            },
        )
        .await
        .map_err(TenantServiceError::Repository)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TenantServiceError {
    #[error(transparent)]
    Domain(#[from] TenantError),
    #[error(transparent)]
    Identity(#[from] IdentityAccessError),
    #[error(transparent)]
    Repository(#[from] TenantRepositoryError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("tenant query is invalid")]
    InvalidQuery,
    #[error("tenant status transition is invalid")]
    InvalidTransition,
    #[error("tenant not found")]
    NotFound,
}
