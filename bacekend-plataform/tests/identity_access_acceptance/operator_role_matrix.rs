use platform_contracts::identity_access::{
    AuthorizeOperatorOperationRequest, identity_access_client::IdentityAccessClient,
};
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use tonic::transport::Channel;
use uuid::Uuid;

pub struct OperatorRoleMatrixContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub authorization_client: &'a mut IdentityAccessClient<Channel>,
    pub operator_id: Uuid,
    pub matrix_operator_session_cookie: &'a str,
    pub matrix_operator_csrf_cookie: &'a str,
    pub created_tenant_id: Uuid,
    pub http_addr: SocketAddr,
}

pub async fn verify(context: OperatorRoleMatrixContext<'_>) {
    let OperatorRoleMatrixContext {
        client,
        migration_pool,
        authorization_client,
        operator_id,
        matrix_operator_session_cookie,
        matrix_operator_csrf_cookie,
        created_tenant_id,
        http_addr,
    } = context;
    // Exercise operator grants and real tenant administration endpoints for
    // platform_owner, platform_security, and platform_support.
    let operator_role_matrix = [
        (
            "platform_owner",
            &[
                "tenants.read",
                "tenants.verify",
                "tenants.activate",
                "tenants.suspend",
                "tenants.close",
                "operators.bootstrap",
            ][..],
        ),
        (
            "platform_security",
            &[
                "tenants.read",
                "tenants.verify",
                "tenants.activate",
                "tenants.suspend",
                "tenants.close",
            ][..],
        ),
        ("platform_support", &["tenants.read"][..]),
    ];
    for (role, allowed_permissions) in operator_role_matrix {
        sqlx::query("UPDATE identity_access.operator_users SET role=$1 WHERE id=$2")
            .bind(role)
            .bind(operator_id)
            .execute(migration_pool)
            .await
            .expect("operator role can be switched for isolated acceptance");
        for permission in [
            "tenants.read",
            "tenants.verify",
            "tenants.activate",
            "tenants.suspend",
            "tenants.close",
            "operators.bootstrap",
        ] {
            let decision = authorization_client
                .authorize_operator_operation(AuthorizeOperatorOperationRequest {
                    session_token: matrix_operator_session_cookie.to_owned(),
                    csrf_token: matrix_operator_csrf_cookie.to_owned(),
                    permission: permission.to_owned(),
                    correlation_id: format!("corr_operator_role_{role}_{permission}"),
                })
                .await;
            if allowed_permissions.contains(&permission) {
                assert!(
                    decision.is_ok(),
                    "role {role} must be allowed permission {permission}"
                );
            } else {
                assert_eq!(
                    decision
                        .expect_err("ungranted operator permission is denied")
                        .code(),
                    tonic::Code::PermissionDenied,
                    "role {role} must be denied permission {permission}"
                );
            }
        }
        let operator_tenant_list = client
            .get(format!("http://{http_addr}/v1/platform/tenants?limit=10&offset=0"))
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .send()
            .await
            .expect("operator tenant listing responds for each role");
        assert_eq!(
            operator_tenant_list.status(),
            StatusCode::OK,
            "all operator roles have tenants.read"
        );
        let operator_tenant_detail = client
            .get(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}"
            ))
            .header(
                header::COOKIE,
                format!("payvia_platform_session={matrix_operator_session_cookie}; payvia_platform_csrf={matrix_operator_csrf_cookie}"),
            )
            .send()
            .await
            .expect("operator tenant detail endpoint responds for each role");
        assert_eq!(
            operator_tenant_detail.status(),
            StatusCode::OK,
            "operator role {role} can read tenant details"
        );
    }
}
