use redis::aio::ConnectionManager;
use sqlx::PgPool;

mod api_key_authorization;
mod api_keys;
mod authorization;
mod rate_limits;
mod registration;
mod session_creation;
mod session_revocation;
mod session_validation;
mod support;
mod types;

pub use types::*;

#[derive(Clone)]
pub struct IdentityService {
    pub(super) pool: PgPool,
    pub(super) redis: ConnectionManager,
}

impl IdentityService {
    pub fn new(pool: PgPool, redis: ConnectionManager) -> Self {
        Self { pool, redis }
    }
}
