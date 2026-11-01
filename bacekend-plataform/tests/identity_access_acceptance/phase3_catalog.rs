use std::net::SocketAddr;

use chrono::Utc;
use reqwest::header;
use reqwest::StatusCode;
use uuid::Uuid;

pub async fn configure(
    client: &reqwest::Client,
    http_addr: SocketAddr,
    origin: &str,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
    tenant_id: Uuid,
) -> String {
    let phase3_slug = format!("phase3-{}", Uuid::new_v4().simple());
    let phase3_plan = client
        .post(format!("http://{http_addr}/v1/platform/catalog/plans"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_plan_create")
        .json(&serde_json::json!({
            "slug": phase3_slug,
            "name": "Phase 3 Acceptance",
            "description": "Real BFF to gRPC catalog acceptance"
        }))
        .send()
        .await
        .expect("catalog plan creation traverses the real BFF and gRPC module");
    assert_eq!(phase3_plan.status(), StatusCode::CREATED);
    let phase3_plan_body: serde_json::Value = phase3_plan
        .json()
        .await
        .expect("catalog plan response is valid JSON");
    let phase3_plan_id = phase3_plan_body["plan_id"]
        .as_str()
        .expect("catalog plan id is returned")
        .to_owned();
    
    let phase3_effective_from = (Utc::now() - chrono::Duration::seconds(30)).to_rfc3339();
    let phase3_price = client
        .post(format!(
            "http://{http_addr}/v1/platform/catalog/plans/{phase3_plan_id}/prices"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_price_publish")
        .json(&serde_json::json!({
            "plan_id": phase3_plan_id,
            "currency": "USD",
            "amount_minor": 1200,
            "period_unit": "month",
            "interval_count": 1,
            "effective_from": phase3_effective_from,
            "effective_until": null
        }))
        .send()
        .await
        .expect("price publication traverses the real BFF and gRPC module");
    assert_eq!(phase3_price.status(), StatusCode::CREATED);
    let phase3_price_body: serde_json::Value = phase3_price
        .json()
        .await
        .expect("published price response is valid JSON");
    let phase3_price_id = phase3_price_body["price_version_id"]
        .as_str()
        .expect("published price version id is returned")
        .to_owned();
    
    let phase3_capabilities = client
        .post(format!("http://{http_addr}/v1/platform/entitlements/capabilities"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_capabilities")
        .json(&serde_json::json!({
            "price_version_id": phase3_price_id,
            "capabilities": [{
                "capability_key": "payments.monthly",
                "enabled": true,
                "limit_value": 2,
                "unit": "operations",
                "overage_policy": "deny",
                "environment": "all",
                "quota_period": "month"
            }]
        }))
        .send()
        .await
        .expect("capability configuration traverses the real BFF and gRPC module");
    assert_eq!(phase3_capabilities.status(), StatusCode::OK);
    let phase3_capabilities_body: serde_json::Value = phase3_capabilities
        .json()
        .await
        .expect("capability configuration response is valid JSON");
    assert_eq!(phase3_capabilities_body["capabilities_saved"], 1);
    
    let phase3_assignment = client
        .post(format!("http://{http_addr}/v1/platform/entitlements/assignments"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!(
                "payvia_platform_session={operator_session_cookie}; payvia_platform_csrf={operator_csrf_cookie}"
            ),
        )
        .header("x-csrf-token", operator_csrf_cookie)
        .header("x-correlation-id", "corr_phase3_assignment")
        .json(&serde_json::json!({
            "tenant_id": tenant_id.to_string(),
            "price_version_id": phase3_price_id,
            "effective_from": phase3_effective_from,
            "effective_until": null
        }))
        .send()
        .await
        .expect("tenant plan assignment traverses the real BFF and gRPC module");
    assert_eq!(phase3_assignment.status(), StatusCode::CREATED);
    phase3_price_id
}
