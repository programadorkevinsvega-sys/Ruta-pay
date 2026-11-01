use super::*;
use platform_contracts::realtime::{
    SubscribeEventsRequest, realtime_gateway_client::RealtimeGatewayClient,
};
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use tokio::time::{Duration, timeout};
use tonic::transport::Channel;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    realtime_acceptance_client: &mut RealtimeGatewayClient<Channel>,
    migration_pool: &PgPool,
    http_addr: SocketAddr,
    origin: &str,
    session_cookie: &str,
    csrf_cookie: &str,
    registered_user_id: Uuid,
    created_tenant_id: Uuid,
    replay_cursor: u64,
) {
    let mut tenant_realtime_subscription_client = realtime_acceptance_client.clone();
    let mut tenant_realtime_subscription = tenant_realtime_subscription_client
        .subscribe_events(SubscribeEventsRequest {
            session_token: session_cookie.to_owned(),
            csrf_token: csrf_cookie.to_owned(),
            audience: "tenant".to_owned(),
            tenant_id: created_tenant_id.to_string(),
            after_cursor: replay_cursor,
            correlation_id: "corr_realtime_tenant_revocation_subscription".to_owned(),
        })
        .await
        .expect("authorized tenant can subscribe to its realtime stream")
        .into_inner();

    let logout = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", csrf_cookie)
        .header("x-correlation-id", "corr_identity_logout")
        .send()
        .await
        .expect("valid logout response arrives");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let tenant_revocation_status = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match tenant_realtime_subscription.message().await {
                Err(status) => break status,
                Ok(Some(_event)) => continue,
                Ok(None) => {
                    panic!("tenant realtime subscription must report authorization revocation")
                }
            }
        }
    })
    .await
    .expect("tenant realtime subscription revalidates session revocation");
    assert_eq!(
        tenant_revocation_status.code(),
        tonic::Code::Unauthenticated
    );
    let tenant_logout_audited: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_access.audit_events WHERE actor_id = $1 AND action = 'identity.session.revoked' AND correlation_id = 'corr_identity_logout')",
    )
    .bind(registered_user_id)
    .fetch_one(migration_pool)
    .await
    .expect("tenant session revocation audit is queryable");
    assert!(
        tenant_logout_audited,
        "tenant session revocation is auditable"
    );
    let replay = client
        .delete(format!("http://{http_addr}/v1/identity/session"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
        )
        .header("x-csrf-token", csrf_cookie)
        .send()
        .await
        .expect("revoked session replay is rejected");
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}
