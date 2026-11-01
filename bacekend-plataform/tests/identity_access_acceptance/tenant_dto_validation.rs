use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct TenantDtoValidationContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub second_session_cookie: &'a str,
    pub second_csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
    pub tenant_before_invalid_transitions: (String, i64),
}

pub async fn verify(context: TenantDtoValidationContext<'_>) {
    let TenantDtoValidationContext {
        client,
        migration_pool,
        http_addr,
        origin,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        second_session_cookie,
        second_csrf_cookie,
        created_tenant_id,
        tenant_before_invalid_transitions,
    } = context;
    for invalid_query in ["limit=0", "limit=101", "offset=-1", "limit=not-a-number"] {
        let invalid_list = client
            .get(format!("http://{http_addr}/v1/platform/tenants?{invalid_query}"))
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .send()
            .await
            .expect("invalid tenant-list pagination receives an HTTP response");
        assert_eq!(
            invalid_list.status(),
            StatusCode::BAD_REQUEST,
            "invalid pagination query {invalid_query} must be rejected"
        );
    }
    let oversized_tenant_query = client
        .get(format!(
            "http://{http_addr}/v1/platform/tenants?query={}",
            "q".repeat(121)
        ))
        .header(
            header::COOKIE,
            format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
        )
        .send()
        .await
        .expect("oversized tenant search query receives an HTTP response");
    assert_eq!(oversized_tenant_query.status(), StatusCode::BAD_REQUEST);

    let tenants_before_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(migration_pool)
            .await
            .expect("tenant count before invalid DTO checks is queryable");
    let invalid_tenant_values = [
        serde_json::json!({"display_name": "A", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "a".repeat(65), "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "passport", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "-12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "1".repeat(21)}),
    ];
    for (index, invalid_body) in invalid_tenant_values.into_iter().enumerate() {
        let response = client
            .post(format!("http://{http_addr}/v1/tenants"))
            .header(header::ORIGIN, origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
            )
            .header("x-csrf-token", second_csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("invalid tenant registration values receive an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "invalid tenant registration value case {index} must fail validation"
        );
    }
    let invalid_tenant_types = [
        serde_json::json!({"display_name": 7, "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": 7, "legal_identification_type": "nit", "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": 7, "legal_identification_number": "12345"}),
        serde_json::json!({"display_name": "Valid tenant", "slug": "valid-slug", "legal_identification_type": "nit", "legal_identification_number": 7}),
    ];
    for (index, invalid_body) in invalid_tenant_types.into_iter().enumerate() {
        let response = client
            .post(format!("http://{http_addr}/v1/tenants"))
            .header(header::ORIGIN, origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
            )
            .header("x-csrf-token", second_csrf_cookie)
            .json(&invalid_body)
            .send()
            .await
            .expect("incorrect tenant DTO field type receives an HTTP response");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong tenant DTO type case {index} must be rejected by the HTTP extractor"
        );
    }
    let tenants_after_invalid_dtos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_registry.tenants")
            .fetch_one(migration_pool)
            .await
            .expect("tenant count after invalid DTO checks is queryable");
    assert_eq!(
        tenants_after_invalid_dtos, tenants_before_invalid_dtos,
        "invalid tenant values and types must not create tenant rows"
    );

    let invalid_tenant_dto = client
        .post(format!("http://{http_addr}/v1/tenants"))
        .header(header::ORIGIN, origin)
        .header(
            header::COOKIE,
            format!("payvia_tenant_session={second_session_cookie}; payvia_tenant_csrf={second_csrf_cookie}"),
        )
        .header("x-csrf-token", second_csrf_cookie)
        .json(&serde_json::json!({
            "display_name": "Invalid tenant DTO",
            "slug": 42,
            "legal_identification_type": "nit",
            "legal_identification_number": "123456"
        }))
        .send()
        .await
        .expect("wrong tenant DTO field type is rejected");
    assert_eq!(
        invalid_tenant_dto.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let tenant_after_invalid_transitions: (String, i64) =
        sqlx::query_as("SELECT status,status_version FROM tenant_registry.tenants WHERE id=$1")
            .bind(created_tenant_id)
            .fetch_one(migration_pool)
            .await
            .expect("tenant state after invalid transitions is queryable");
    assert_eq!(
        tenant_after_invalid_transitions, tenant_before_invalid_transitions,
        "invalid reason and stale version leave tenant state unchanged"
    );
}
