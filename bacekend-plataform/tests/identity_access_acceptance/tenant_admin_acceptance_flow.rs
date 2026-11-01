use platform_contracts::identity_access::identity_access_client::IdentityAccessClient;
use reqwest::Client;
use serde_json::Value;
use sqlx::PgPool;
use std::net::SocketAddr;
use tonic::transport::Channel;
use uuid::Uuid;

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    authorization_client: &mut IdentityAccessClient<Channel>,
    http_addr: SocketAddr,
    origin: &str,
    matrix_operator_session_cookie: &str,
    matrix_operator_csrf_cookie: &str,
    operator_session_cookie: &str,
    operator_csrf_cookie: &str,
    operator_id: Uuid,
    created_tenant_id: Uuid,
    registered_user_id: Uuid,
    second_session_cookie: &str,
    second_csrf_cookie: &str,
    second_tenant_id: Uuid,
    tenant_status_version: Value,
) {
    operator_role_matrix::verify(operator_role_matrix::OperatorRoleMatrixContext {
        client: &client,
        migration_pool: &migration_pool,
        authorization_client: &mut authorization_client,
        operator_id,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        created_tenant_id,
        http_addr,
    })
    .await;

    tenant_status_lifecycle::verify(tenant_status_lifecycle::TenantStatusLifecycleContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
    })
    .await;

    let (tenant_before_invalid_transitions, current_status_version) =
        tenant_status_input_validation::verify(
            tenant_status_input_validation::TenantStatusInputContext {
                client: &client,
                migration_pool: &migration_pool,
                http_addr,
                origin: &origin,
                matrix_operator_session_cookie: &matrix_operator_session_cookie,
                matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
                created_tenant_id,
            },
        )
        .await;
    tenant_status_version_validation::verify(
        tenant_status_version_validation::TenantStatusVersionContext {
            client: &client,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            created_tenant_id,
            current_status_version,
        },
    )
    .await;
    tenant_dto_validation::verify(tenant_dto_validation::TenantDtoValidationContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        second_session_cookie: &second_session_cookie,
        second_csrf_cookie: &second_csrf_cookie,
        created_tenant_id,
        tenant_before_invalid_transitions,
    })
    .await;

    secondary_tenant_lifecycle::verify(
        secondary_tenant_lifecycle::SecondaryTenantLifecycleContext {
            client: &client,
            migration_pool: &migration_pool,
            http_addr,
            origin: &origin,
            matrix_operator_session_cookie: &matrix_operator_session_cookie,
            matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
            operator_id,
            second_tenant_id,
        },
    )
    .await;

    session_security::verify(session_security::SessionSecurityContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        session_cookie: &session_cookie,
        csrf_cookie: &csrf_cookie,
        operator_session_cookie: &operator_session_cookie,
        operator_csrf_cookie: &operator_csrf_cookie,
        second_session_cookie: &second_session_cookie,
        second_csrf_cookie: &second_csrf_cookie,
        second_tenant_id,
        created_tenant_id,
        tenant_status_version: tenant_admin_status_body["status_version"].clone(),
    })
    .await;

    tenant_status_race::verify(tenant_status_race::TenantStatusRaceContext {
        client: &client,
        migration_pool: &migration_pool,
        http_addr,
        origin: &origin,
        matrix_operator_session_cookie: &matrix_operator_session_cookie,
        matrix_operator_csrf_cookie: &matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
        current_status_version,
    })
    .await;
}
