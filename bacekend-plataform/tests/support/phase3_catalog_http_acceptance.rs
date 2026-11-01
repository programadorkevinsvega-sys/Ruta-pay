use reqwest::{Client, StatusCode, header};
use std::net::SocketAddr;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    http_addr: SocketAddr,
    origin: &str,
    session: &str,
    csrf: &str,
) {
    let base = format!("http://{http_addr}");
    let cookies = format!(
        "payvia_platform_session={session}; payvia_platform_csrf={csrf}"
    );
    let correlation = format!("phase3_catalog_http_{}", Uuid::new_v4().simple());
    let slug = format!("phase3-{}", Uuid::new_v4().simple());

    let created = client
        .post(format!("{base}/v1/platform/catalog/plans"))
        .header(header::ORIGIN, origin)
        .header(header::COOKIE, &cookies)
        .header("x-csrf-token", csrf)
        .header("x-correlation-id", format!("{correlation}_create"))
        .json(&serde_json::json!({
            "slug": slug,
            "name": "Phase 3 HTTP acceptance",
            "description": "BFF to gRPC to catalog module"
        }))
        .send()
        .await
        .expect("catalog plan creation reaches BFF");
    assert_eq!(created.status(), StatusCode::CREATED);
    let plan: serde_json::Value = created
        .json()
        .await
        .expect("catalog plan response is valid JSON");
    let plan_id = plan["plan_id"]
        .as_str()
        .expect("catalog plan identifier is present")
        .to_owned();

    let now = chrono::Utc::now();
    let price = client
        .post(format!("{base}/v1/platform/catalog/plans/{plan_id}/prices"))
        .header(header::ORIGIN, origin)
        .header(header::COOKIE, &cookies)
        .header("x-csrf-token", csrf)
        .header("x-correlation-id", format!("{correlation}_price"))
        .json(&serde_json::json!({
            "plan_id": plan_id,
            "currency": "USD",
            "amount_minor": 1200,
            "period_unit": "month",
            "interval_count": 1,
            "effective_from": (now - chrono::Duration::minutes(1)).to_rfc3339(),
            "effective_until": (now + chrono::Duration::days(30)).to_rfc3339()
        }))
        .send()
        .await
        .expect("price publication reaches catalog gRPC module");
    assert_eq!(price.status(), StatusCode::CREATED);
    let price: serde_json::Value = price
        .json()
        .await
        .expect("published price response is valid JSON");
    let price_id = price["price_version_id"]
        .as_str()
        .expect("published price identifier is present")
        .to_owned();

    let capabilities = client
        .post(format!("{base}/v1/platform/entitlements/capabilities"))
        .header(header::ORIGIN, origin)
        .header(header::COOKIE, &cookies)
        .header("x-csrf-token", csrf)
        .header("x-correlation-id", format!("{correlation}_capabilities"))
        .json(&serde_json::json!({
            "price_version_id": price_id,
            "capabilities": [{
                "capability_key": "api.requests",
                "enabled": true,
                "limit_value": 100,
                "unit": "requests",
                "overage_policy": "deny",
                "environment": "all",
                "quota_period": "month"
            }]
        }))
        .send()
        .await
        .expect("capability configuration reaches entitlement gRPC module");
    assert_eq!(capabilities.status(), StatusCode::OK);
    let capabilities: serde_json::Value = capabilities
        .json()
        .await
        .expect("capability response is valid JSON");
    assert_eq!(capabilities["capabilities_saved"], 1);

    let plans = client
        .get(format!("{base}/v1/platform/catalog/plans"))
        .header(header::ORIGIN, origin)
        .header(header::COOKIE, &cookies)
        .header("x-csrf-token", csrf)
        .header("x-correlation-id", format!("{correlation}_list"))
        .send()
        .await
        .expect("catalog plan list reaches BFF");
    assert_eq!(plans.status(), StatusCode::OK);
    let plans: Vec<serde_json::Value> = plans
        .json()
        .await
        .expect("catalog plan list is valid JSON");
    assert!(
        plans.iter().any(|item| item["plan_id"] == plan_id),
        "newly created plan is returned through the HTTP catalog API"
    );
}
