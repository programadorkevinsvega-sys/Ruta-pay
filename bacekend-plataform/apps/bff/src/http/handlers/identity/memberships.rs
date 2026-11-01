use super::*;

#[utoipa::path(get, path = "/v1/identity/memberships", responses((status = 200, body = TenantMembershipListResponse), (status = 401, body = ErrorEnvelope)))]
pub async fn tenant_memberships(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let (Some(session_token), Some(csrf_token)) = (
        cookie_value(&headers, SESSION_COOKIE),
        cookie_value(&headers, CSRF_COOKIE),
    ) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let started = std::time::Instant::now();
    match IdentityAccessClient::list_tenant_memberships(
        &state.identity_client,
        ListTenantMembershipsRequest {
            session_token,
            csrf_token,
            correlation_id: correlation.clone(),
        },
        &correlation,
    )
    .await
    {
        Ok(result) => Json(TenantMembershipListResponse {
            memberships: result
                .memberships
                .into_iter()
                .map(|membership| TenantMembershipResponse {
                    tenant_id: membership.tenant_id,
                    role: membership.role,
                    status: membership.membership_status,
                })
                .collect(),
        })
        .into_response(),
        Err(status) => grpc_error(
            status,
            "list_tenant_memberships",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}
