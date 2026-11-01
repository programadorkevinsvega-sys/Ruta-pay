use std::net::SocketAddr;

use platform_contracts::{
    entitlements_quotas::{ConsumeQuotaRequest, entitlements_quotas_client::EntitlementsQuotasClient},
    identity_access::{AuthorizeApiKeyOperationRequest, identity_access_client::IdentityAccessClient},
};
use reqwest::header;
use reqwest::StatusCode;
use tonic::transport::Channel;
use uuid::Uuid;

pub async fn verify(
    client: &reqwest::Client,
    http_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    tenant_id: Uuid,
    api_key_secret: &str,
    authorization_client: IdentityAccessClient<Channel>,
    entitlements_acceptance_client: EntitlementsQuotasClient<Channel>,
) {
    let consume_scope_authorization = authorization_client
        .authorize_api_key_operation(AuthorizeApiKeyOperationRequest {
            secret: api_key_secret.to_owned(),
            tenant_id: tenant_id.to_string(),
            environment: "live".to_owned(),
            required_scope: "usage.consume".to_owned(),
            correlation_id: "corr_phase3_consume_scope_allowed".to_owned(),
        })
        .await
        .expect("API key explicitly grants the quota consumption scope")
        .into_inner();
    assert_eq!(consume_scope_authorization.key_id, api_key_id.to_string());
    
    let phase3_entitlement_before = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{tenant_id}/entitlements?environment=live"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant entitlement list traverses the real BFF and gRPC module");
    assert_eq!(phase3_entitlement_before.status(), StatusCode::OK);
    let phase3_entitlements: serde_json::Value = phase3_entitlement_before
        .json()
        .await
        .expect("tenant entitlement list is valid JSON");
    assert_eq!(
        phase3_entitlements.as_array().map(std::vec::Vec::len),
        Some(1),
        "tenant receives exactly the capabilities assigned to its plan"
    );
    assert_eq!(phase3_entitlements[0]["capability_key"], "payments.monthly");
    assert_eq!(phase3_entitlements[0]["consumed"], 0);
    
    let phase3_consumption = entitlements_acceptance_client
        .clone()
        .consume_quota(tonic::Request::new(
            platform_contracts::entitlements_quotas::ConsumeQuotaRequest {
                api_key_secret: api_key_secret.to_owned(),
                tenant_id: tenant_id.to_string(),
                capability_key: "payments.monthly".to_owned(),
                environment: "live".to_owned(),
                quantity: 1,
                idempotency_key: "phase3-grpc-consume-001".to_owned(),
                correlation_id: "corr_phase3_grpc_consume".to_owned(),
            },
        ))
        .await
        .expect("quota consumption requires the real API key authorization gRPC path")
        .into_inner();
    assert!(!phase3_consumption.replayed);
    let phase3_consumed_entitlement = phase3_consumption
        .entitlement
        .expect("quota consumption returns the updated entitlement");
    assert_eq!(phase3_consumed_entitlement.consumed, 1);
    assert_eq!(phase3_consumed_entitlement.remaining, Some(1));
    
    let phase3_replay = entitlements_acceptance_client
        .clone()
        .consume_quota(tonic::Request::new(
            platform_contracts::entitlements_quotas::ConsumeQuotaRequest {
                api_key_secret: api_key_secret.to_owned(),
                tenant_id: tenant_id.to_string(),
                capability_key: "payments.monthly".to_owned(),
                environment: "live".to_owned(),
                quantity: 1,
                idempotency_key: "phase3-grpc-consume-001".to_owned(),
                correlation_id: "corr_phase3_grpc_consume_replay".to_owned(),
            },
        ))
        .await
        .expect("quota consumption replay is idempotent across the gRPC boundary")
        .into_inner();
    assert!(phase3_replay.replayed);
    assert_eq!(
        phase3_replay
            .entitlement
            .expect("idempotent replay returns the recorded entitlement")
            .consumed,
        1
    );
    
    let phase3_entitlement_after = client
        .get(format!(
            "http://{http_addr}/v1/tenants/{tenant_id}/entitlements/payments.monthly?environment=live"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .send()
        .await
        .expect("tenant entitlement detail reflects gRPC quota consumption");
    assert_eq!(phase3_entitlement_after.status(), StatusCode::OK);
    let phase3_entitlement_after_body: serde_json::Value = phase3_entitlement_after
        .json()
        .await
        .expect("updated tenant entitlement is valid JSON");
    assert_eq!(phase3_entitlement_after_body["consumed"], 1);
    assert_eq!(phase3_entitlement_after_body["remaining"], 1);
}
