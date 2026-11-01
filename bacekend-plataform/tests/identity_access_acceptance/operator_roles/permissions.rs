use crate::identity_access_acceptance_context::AcceptanceContext;
use platform_contracts::identity_access::AuthorizeOperatorOperationRequest;
use reqwest::{header, StatusCode};

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
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
        .bind(ctx.operator_id)
        .execute(&ctx.migration_pool)
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
        let decision = ctx.authorization_client
            .authorize_operator_operation(AuthorizeOperatorOperationRequest {
                session_token: ctx.state.matrix_operator_session_cookie.clone(),
                csrf_token: ctx.state.matrix_operator_csrf_cookie.clone(),
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
    let operator_tenant_list = ctx.client
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
    let operator_tenant_detail = ctx.client
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
 // platform_support may read tenants but must not suspend them. Verify that
// the denial leaves both status and version unchanged.}
