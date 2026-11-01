use platform_contracts::identity_access::{
    AuthorizeTenantOperationRequest,
    identity_access_client::IdentityAccessClient,
};
use reqwest::{Client, StatusCode, header};
use sqlx::PgPool;
use std::net::SocketAddr;
use tonic::transport::Channel;
use uuid::Uuid;

pub struct TenantRoleMatrixContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub authorization_client: &'a mut IdentityAccessClient<Channel>,
    pub http_addr: SocketAddr,
    pub session_cookie: &'a str,
    pub csrf_cookie: &'a str,
    pub origin: &'a str,
    pub created_tenant_id: Uuid,
    pub registered_user_id: Uuid,
}

pub async fn verify(context: TenantRoleMatrixContext<'_>) {
    let TenantRoleMatrixContext {
        client,
        migration_pool,
        authorization_client,
        http_addr,
        session_cookie,
        csrf_cookie,
        origin,
        created_tenant_id,
        registered_user_id,
    } = context;
    // Exercise the complete seeded tenant-role permission matrix through the
    // real authorization RPC, then verify the exposed tenant/API-key operations.
    let role_matrix = [
        ("developer", &["tenant.read", "api_keys.manage"][..]),
        (
            "tenant_admin",
            &[
                "tenant.read",
                "api_keys.manage",
                "members.manage",
                "tenant.settings.manage",
            ][..],
        ),
        (
            "tenant_owner",
            &[
                "tenant.read",
                "api_keys.manage",
                "members.manage",
                "tenant.settings.manage",
            ][..],
        ),
        ("viewer", &["tenant.read"][..]),
    ];
    for (role, allowed_permissions) in role_matrix {
        sqlx::query(
            "UPDATE identity_access.tenant_memberships SET role=$1 WHERE tenant_id=$2 AND user_id=$3",
        )
        .bind(role)
        .bind(created_tenant_id)
        .bind(registered_user_id)
        .execute(&migration_pool)
        .await
        .expect("tenant membership role can be switched for isolated acceptance");
    
        for permission in [
            "tenant.read",
            "api_keys.manage",
            "members.manage",
            "tenant.settings.manage",
        ] {
            let decision = authorization_client
                .authorize_tenant_operation(AuthorizeTenantOperationRequest {
                    session_token: session_cookie.clone(),
                    csrf_token: csrf_cookie.clone(),
                    tenant_id: created_tenant_id.to_string(),
                    permission: permission.to_owned(),
                    correlation_id: format!("corr_role_{role}_{permission}"),
                })
                .await;
            if allowed_permissions.contains(&permission) {
                assert!(
                    decision.is_ok(),
                    "role {role} must be allowed permission {permission}: {:?}",
                    decision.err()
                );
            } else {
                assert_eq!(
                    decision
                        .expect_err("ungranted tenant permission is denied")
                        .code(),
                    tonic::Code::PermissionDenied,
                    "role {role} must be denied permission {permission}"
                );
            }
        }
    
        let tenant_read = client
            .get(format!("http://{http_addr}/v1/tenants/{created_tenant_id}"))
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .send()
            .await
            .expect("tenant read operation returns a response for the current role");
        assert_eq!(
            tenant_read.status(),
            StatusCode::OK,
            "role {role} has tenant.read"
        );
    
        let api_key_list_for_role = client
            .get(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .send()
            .await
            .expect("API key list operation returns a response for the current role");
        let can_manage_keys = allowed_permissions.contains(&"api_keys.manage");
        assert_eq!(
            api_key_list_for_role.status(),
            if can_manage_keys {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            },
            "role {role} API key list permission is enforced at HTTP boundary"
        );
    
        let create_key_for_role = client
            .post(format!(
                "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys"
            ))
            .header(header::ORIGIN, &origin)
            .header(
                header::COOKIE,
                format!("payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"),
            )
            .header("x-csrf-token", &csrf_cookie)
            .json(&serde_json::json!({
                "name": format!("Role matrix {role}"),
                "environment": "test",
                "scopes": ["payment_intents.create"]
            }))
            .send()
            .await
            .expect("API key create operation returns a response for the current role");
        assert_eq!(
            create_key_for_role.status(),
            if can_manage_keys {
                StatusCode::CREATED
            } else {
                StatusCode::FORBIDDEN
            },
            "role {role} API key create permission is enforced at HTTP boundary"
        );
        if can_manage_keys {
            let created_for_role: serde_json::Value = create_key_for_role
                .json()
                .await
                .expect("authorized role receives API key metadata");
            let key_id = created_for_role["key_id"]
                .as_str()
                .expect("role matrix key id is present");
            let revoke_for_role = client
                .delete(format!(
                    "http://{http_addr}/v1/tenants/{created_tenant_id}/api-keys/{key_id}"
                ))
                .header(header::ORIGIN, &origin)
                .header(
                    header::COOKIE,
                    format!(
                        "payvia_tenant_session={session_cookie}; payvia_tenant_csrf={csrf_cookie}"
                    ),
                )
                .header("x-csrf-token", &csrf_cookie)
                .send()
                .await
                .expect("authorized role can revoke its own tenant API key");
            assert_eq!(revoke_for_role.status(), StatusCode::NO_CONTENT);
        }
    }
    sqlx::query(
        "UPDATE identity_access.tenant_memberships SET role='tenant_owner' WHERE tenant_id=$1 AND user_id=$2",
    )
    .bind(created_tenant_id)
    .bind(registered_user_id)
    .execute(&migration_pool)
    .await
    .expect("tenant owner role is restored after the role matrix");
}
