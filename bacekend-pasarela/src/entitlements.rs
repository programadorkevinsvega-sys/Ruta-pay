use platform_contracts::entitlements_quotas::{
    ConsumeQuotaRequest, ConsumeQuotaResponse, GetTenantEntitlementRequest,
    TenantEntitlementResponse, entitlements_quotas_client::EntitlementsQuotasClient,
};
use tonic::{
    Request, Status,
    transport::{Channel, Endpoint},
};

#[derive(Clone)]
pub struct GatewayEntitlementsClient {
    inner: EntitlementsQuotasClient<Channel>,
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayEntitlementsConnectError {
    #[error("platform entitlement endpoint is invalid: {0}")]
    InvalidEndpoint(String),
    #[error("platform entitlement service is unavailable")]
    Transport(#[from] tonic::transport::Error),
}

impl GatewayEntitlementsClient {
    pub async fn connect(
        endpoint: impl Into<String>,
    ) -> Result<Self, GatewayEntitlementsConnectError> {
        let endpoint = Endpoint::from_shared(endpoint.into())
            .map_err(|error| GatewayEntitlementsConnectError::InvalidEndpoint(error.to_string()))?;
        let channel = endpoint.connect().await?;
        Ok(Self {
            inner: EntitlementsQuotasClient::new(channel),
        })
    }

    /// Sends a tenant-scoped, API-key-authorized quota consumption to the platform.
    /// The secret is passed only in the RPC request and is never logged or retained here.
    pub async fn consume_quota(
        &self,
        request: ConsumeQuotaRequest,
    ) -> Result<ConsumeQuotaResponse, Status> {
        self.inner
            .clone()
            .consume_quota(Request::new(request))
            .await
            .map(tonic::Response::into_inner)
    }

    /// Reads the effective entitlement through the platform-owned authorization boundary.
    pub async fn get_tenant_entitlement(
        &self,
        request: GetTenantEntitlementRequest,
    ) -> Result<TenantEntitlementResponse, Status> {
        self.inner
            .clone()
            .get_tenant_entitlement(Request::new(request))
            .await
            .map(tonic::Response::into_inner)
    }
}
