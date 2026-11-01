use chrono::Utc;
use platform_contracts::{
    entitlements_quotas::{ConsumeQuotaRequest, ConsumeQuotaResponse},
    identity_access::{AuthorizeApiKeyOperationRequest, identity_access_client::IdentityAccessClient},
};
use tonic::{Request, Response, Status};

use crate::application::entitlement_service::ConsumeQuotaCommand;
use super::{EntitlementsQuotasApi, mapping::{entitlement_response, map_error, parse_uuid}};

impl EntitlementsQuotasApi {
        pub(super) async fn consume_quota_handler(
            &self,
            request: Request<ConsumeQuotaRequest>,
        ) -> Result<Response<ConsumeQuotaResponse>, Status> {
            let body = request.into_inner();
            let tenant = parse_uuid(&body.tenant_id)?;
            let correlation_id = body.correlation_id.clone();
            let authorization = self
                .identity
                .clone()
                .authorize_api_key_operation(Request::new(AuthorizeApiKeyOperationRequest {
                    secret: body.api_key_secret,
                    tenant_id: tenant.to_string(),
                    environment: body.environment.clone(),
                    required_scope: "usage.consume".to_owned(),
                    correlation_id: correlation_id.clone(),
                }))
                .await?
                .into_inner();
            if authorization.tenant_id != tenant.to_string()
                || authorization.environment != body.environment
            {
                return Err(Status::permission_denied(
                    "API key is not authorized for this tenant environment",
                ));
            }
            let actor = parse_uuid(&authorization.key_id)?;
            let (entitlement, replayed) = self
                .service
                .consume(ConsumeQuotaCommand {
                    actor,
                    tenant,
                    key: &body.capability_key,
                    environment: &body.environment,
                    quantity: body.quantity,
                    idempotency_key: &body.idempotency_key,
                    now: Utc::now(),
                    correlation_id: &correlation_id,
                })
                .await
                .map_err(map_error)?;
            Ok(Response::new(ConsumeQuotaResponse {
                entitlement: Some(entitlement_response(entitlement)),
                replayed,
            }))
        }
}
