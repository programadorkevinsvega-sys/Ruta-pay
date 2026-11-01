use super::*;

#[utoipa::path(post, path = "/v1/identity/session", request_body = Credentials, responses((status = 200, body = SessionStarted), (status = 401, body = ErrorEnvelope)))]
pub async fn login(
    State(state): State<std::sync::Arc<AppState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Response {
    let correlation = correlation_id(&headers);
    if !valid_origin(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "authorization",
            false,
            correlation,
        );
    }
    let started = std::time::Instant::now();
    match IdentityAccessClient::login(
        &state.identity_client,
        body.email,
        body.password,
        remote.ip().to_string(),
        &correlation,
    )
    .await
    {
        Ok(session) => {
            let mut response = (
                StatusCode::OK,
                Json(SessionStarted {
                    user_id: session.user_id,
                    expires_at: session.expires_at.clone(),
                }),
            )
                .into_response();
            if append_cookie(
                &mut response,
                format!(
                    "{SESSION_COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=43200",
                    session.session_token
                ),
            )
            .is_err()
                || append_cookie(
                    &mut response,
                    format!(
                        "{CSRF_COOKIE}={}; Path=/; Secure; SameSite=Lax; Max-Age=43200",
                        session.csrf_token
                    ),
                )
                .is_err()
            {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session_cookie_failed",
                    "internal",
                    false,
                    correlation,
                );
            }
            response
        }
        Err(status) => grpc_error(
            status,
            "create_tenant_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path = "/v1/identity/session", responses((status = 200, body = TenantSessionResponse), (status = 401, body = ErrorEnvelope)))]
pub async fn tenant_session(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some(session_token) = cookie_value(&headers, SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let Some(csrf_token) = cookie_value(&headers, CSRF_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let started = std::time::Instant::now();
    match IdentityAccessClient::validate_session(
        &state.identity_client,
        session_token,
        csrf_token,
        &correlation,
    )
    .await
    {
        Ok(session) => Json(TenantSessionResponse {
            user_id: session.user_id,
        })
        .into_response(),
        Err(status) => grpc_error(
            status,
            "validate_tenant_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}
