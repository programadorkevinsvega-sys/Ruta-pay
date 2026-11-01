use super::*;
use platform_contracts::identity_access::identity_access_client::IdentityAccessClient;
use reqwest::Client;
use serde_json::Value;
use sqlx::PgPool;
use std::net::SocketAddr;
use tonic::transport::Channel;
use uuid::Uuid;

pub struct TenantAdminAcceptanceContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub authorization_client: &'a mut IdentityAccessClient<Channel>,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub operator_session_cookie: &'a str,
    pub operator_csrf_cookie: &'a str,
    pub operator_id: Uuid,
    pub created_tenant_id: Uuid,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub second_session_cookie: &'a str,
    pub second_csrf_cookie: &'a str,
    pub second_tenant_id: Uuid,
    pub tenant_status_version: Value,
}

pub async fn verify(context: TenantAdminAcceptanceContext<'_>) {
    let TenantAdminAcceptanceContext {
        client, migration_pool, authorization_client, http_addr, origin,
        matrix_operator_session_cookie, matrix_operator_csrf_cookie,
        operator_session_cookie, operator_csrf_cookie, operator_id,
        created_tenant_id, session_cookie, csrf_cookie, second_session_cookie,
        second_csrf_cookie, second_tenant_id, tenant_status_version,
    } = context;
    operator_role_matrix::verify(operator_role_matrix::OperatorRoleMatrixContext {
        client,
        migration_pool,
        authorization_client,
        operator_id,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        created_tenant_id,
        http_addr,
    })
    .await;

    tenant_status_lifecycle::verify(tenant_status_lifecycle::TenantStatusLifecycleContext {
        client,
        migration_pool,
        http_addr,
        origin,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
    })
    .await;

    let (tenant_before_invalid_transitions, current_status_version) =
        tenant_status_input_validation::verify(
            tenant_status_input_validation::TenantStatusInputContext {
                client,
                migration_pool,
                http_addr,
                origin,
                matrix_operator_session_cookie,
                matrix_operator_csrf_cookie,
                created_tenant_id,
            },
        )
        .await;
    tenant_status_version_validation::verify(
        tenant_status_version_validation::TenantStatusVersionContext {
            client,
            http_addr,
            origin,
            matrix_operator_session_cookie,
            matrix_operator_csrf_cookie,
            created_tenant_id,
            current_status_version,
        },
    )
    .await;
    tenant_dto_validation::verify(tenant_dto_validation::TenantDtoValidationContext {
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
    })
    .await;

    secondary_tenant_lifecycle::verify(
        secondary_tenant_lifecycle::SecondaryTenantLifecycleContext {
            client,
            migration_pool,
            http_addr,
            origin,
            matrix_operator_session_cookie,
            matrix_operator_csrf_cookie,
            operator_id,
            second_tenant_id,
        },
    )
    .await;

    session_security::verify(session_security::SessionSecurityContext {
        client,
        migration_pool,
        http_addr,
        origin,
        session_cookie,
        csrf_cookie,
        operator_session_cookie,
        operator_csrf_cookie,
        second_session_cookie,
        second_csrf_cookie,
        second_tenant_id,
        created_tenant_id,
        tenant_status_version,
    })
    .await;

    tenant_status_race::verify(tenant_status_race::TenantStatusRaceContext {
        client,
        migration_pool,
        http_addr,
        origin,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        operator_id,
        created_tenant_id,
        current_status_version,
    })
    .await;
}
