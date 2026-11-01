mod event_ingestion;
mod server;
mod support;

pub use event_ingestion::persist_and_publish;

use platform_contracts::identity_access::{
    AuthorizeTenantOperationRequest, ValidateOperatorSessionRequest,
    identity_access_client::IdentityAccessClient,
};
use sqlx::PgPool;
use tonic::{Request, Status};
use uuid::Uuid;

const SIGNAL_CHANNEL: &str = "payvia:platform:realtime:v1";
const MAX_REPLAY_LIMIT: u32 = 250;
const BATCH_SIZE: i64 = 100;

#[derive(Clone)]
pub struct RealtimeGatewayApi {
    pool: PgPool,
    redis_client: redis::Client,
    identity_client: IdentityAccessClient<tonic::transport::Channel>,
}

impl RealtimeGatewayApi {
    pub fn new(
        pool: PgPool,
        redis_client: redis::Client,
        identity_client: IdentityAccessClient<tonic::transport::Channel>,
    ) -> Self {
        Self { pool, redis_client, identity_client }
    }

    async fn authorize(
        &self,
        token: &str,
        csrf: &str,
        audience: &str,
        tenant_id: Option<Uuid>,
        correlation_id: &str,
    ) -> Result<(), Status> {
        let mut identity = self.identity_client.clone();
        match audience {
            "platform" => identity
                .validate_operator_session(ValidateOperatorSessionRequest {
                    session_token: token.to_owned(),
                    csrf_token: csrf.to_owned(),
                    correlation_id: correlation_id.to_owned(),
                })
                .await
                .map(|_| ()),
            "tenant" => {
                let tenant_id = tenant_id
                    .ok_or_else(|| Status::invalid_argument("tenant scope is required"))?;
                identity
                    .authorize_tenant_operation(AuthorizeTenantOperationRequest {
                        session_token: token.to_owned(),
                        csrf_token: csrf.to_owned(),
                        tenant_id: tenant_id.to_string(),
                        permission: "tenant.read".to_owned(),
                        correlation_id: correlation_id.to_owned(),
                    })
                    .await
                    .map(|_| ())
            }
            _ => Err(Status::invalid_argument("realtime audience is invalid")),
        }
    }
}
