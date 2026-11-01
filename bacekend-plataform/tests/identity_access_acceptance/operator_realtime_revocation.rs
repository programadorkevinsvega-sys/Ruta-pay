use platform_contracts::realtime::{
    SubscribeEventsRequest, realtime_gateway_client::RealtimeGatewayClient,
};
use reqwest::{Client, StatusCode, header};
use std::net::SocketAddr;
use tokio::time::{Duration, timeout};
use tonic::transport::Channel;

pub async fn verify(
    client: &Client,
    realtime_acceptance_client: &mut RealtimeGatewayClient<Channel>,
    http_addr: SocketAddr,
    origin: &str,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
) {
    // Keep an authenticated realtime subscription open while the operator
    // session is revoked. The stream must re-check authorization and terminate.
    let mut realtime_subscription_client = realtime_acceptance_client.clone();
    let realtime_subscription = realtime_subscription_client
        .subscribe_events(SubscribeEventsRequest {
            session_token: operator_session_cookie.to_owned(),
            csrf_token: operator_csrf_cookie.to_owned(),
            audience: "platform".to_owned(),
            tenant_id: String::new(),
            after_cursor: 0,
            correlation_id: "corr_realtime_revocation_subscription".to_owned(),
        })
        .await
        .expect("authorized operator can subscribe to platform realtime")
        .into_inner();

    let operator_logout = client
        .delete(format!("http://{http_addr}/v1/platform/identity/session"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_operator_logout")
        .send()
        .await
        .expect("operator logout response arrives");
    assert_eq!(operator_logout.status(), StatusCode::NO_CONTENT);
    let mut revoked_subscription = realtime_subscription;
    let realtime_revocation_status = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match revoked_subscription.message().await {
                Err(status) => break status,
                Ok(Some(_event)) => continue,
                Ok(None) => panic!("realtime subscription must report authorization revocation"),
            }
        }
    })
    .await
    .expect("realtime subscription revalidates operator authorization within 20 seconds");
    assert_eq!(
        realtime_revocation_status.code(),
        tonic::Code::Unauthenticated
    );
}
