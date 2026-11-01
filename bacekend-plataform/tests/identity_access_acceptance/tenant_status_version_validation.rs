use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct TenantStatusVersionContext<'a> {
    pub client: &'a Client,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
    pub current_status_version: i64,
}

pub async fn verify(context: TenantStatusVersionContext<'_>) {
    let TenantStatusVersionContext {
        client,
        http_addr,
        origin,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        created_tenant_id,
        current_status_version,
    } = context;
    let stale_status_version = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .header("x-correlation-id", "corr_tenant_status_stale_version")
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "This stale transition must not be applied",
            "expected_version": current_status_version - 1
        }))
        .send()
        .await
        .expect("stale tenant status version is rejected");
    assert_eq!(stale_status_version.status(), StatusCode::CONFLICT);

    let zero_expected_version = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "A version must be positive",
            "expected_version": -1
        }))
        .send()
        .await
        .expect("negative expected version is rejected as invalid input");
    assert_eq!(zero_expected_version.status(), StatusCode::BAD_REQUEST);

    let wrong_version_type = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "suspended",
            "reason": "The version must be an integer",
            "expected_version": "not-an-integer"
        }))
        .send()
        .await
        .expect("wrong expected_version JSON type is rejected");
    assert_eq!(
        wrong_version_type.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let invalid_status_dto_types = [
        serde_json::json!({
            "status": 7,
            "reason": "Status must be a string",
            "expected_version": current_status_version
        }),
        serde_json::json!({
            "status": "suspended",
            "reason": 7,
            "expected_version": current_status_version
        }),
    ];
    for (index, invalid_body) in invalid_status_dto_types.into_iter().enumerate() {
        let response = client
            .post(format!(
                "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
            ))
            .header(header::ORIGIN, origin)
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .header("x-csrf-token", &matrix_operator_csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("wrong lifecycle DTO field type receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong lifecycle DTO type case {index} must be rejected"
        );
    }

    let invalid_status_value = client
        .post(format!(
            "http://{http_addr}/v1/platform/tenants/{created_tenant_id}/status"
        ))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .header("x-csrf-token", &matrix_operator_csrf_cookie)
        .json(&serde_json::json!({
            "status": "unknown-state",
            "reason": "Unknown states must not be accepted",
            "expected_version": current_status_version
        }))
        .send()
        .await
        .expect("unknown lifecycle state is rejected");
    assert_eq!(invalid_status_value.status(), StatusCode::BAD_REQUEST);
}
