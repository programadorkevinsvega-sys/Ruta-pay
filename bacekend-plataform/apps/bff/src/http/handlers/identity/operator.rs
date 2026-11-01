use super::*;

#[utoipa::path(post, path = "/v1/platform/identity/session", request_body = Credentials, responses((status = 200, body = OperatorSessionResponse), (status = 401, body = ErrorEnvelope)))]
pub async fn operator_login(
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
    match IdentityAccessClient::operator_login(
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
                Json(OperatorSessionResponse {
                    operator_id: session.user_id.clone(),
                }),
            )
                .into_response();
            if append_cookie(
                &mut response,
                format!(
                    "{OPERATOR_SESSION_COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=43200",
                    session.session_token
                ),
            )
            .is_err()
                || append_cookie(
                    &mut response,
                    format!(
                        "{OPERATOR_CSRF_COOKIE}={}; Path=/; Secure; SameSite=Lax; Max-Age=43200",
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
            "create_operator_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(get, path = "/v1/platform/identity/session", responses((status = 200, body = OperatorSessionResponse), (status = 401, body = ErrorEnvelope)))]
pub async fn operator_session(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    let Some(session_token) = cookie_value(&headers, OPERATOR_SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let Some(csrf_token) = cookie_value(&headers, OPERATOR_CSRF_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    match IdentityAccessClient::validate_operator_session(
        &state.identity_client,
        session_token,
        csrf_token,
        &correlation,
    )
    .await
    {
        Ok(session) => Json(OperatorSessionResponse {
            operator_id: session.user_id,
        })
        .into_response(),
        Err(status) => grpc_error(status, "validate_operator_session", &correlation, 0),
    }
}

#[utoipa::path(delete, path = "/v1/platform/identity/session", responses((status = 204), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn operator_logout(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let correlation = correlation_id(&headers);
    if !valid_origin(&headers) || !valid_csrf(&headers, OPERATOR_CSRF_COOKIE) {
        return error(
            StatusCode::FORBIDDEN,
            "csrf_validation_failed",
            "authorization",
            false,
            correlation,
        );
    }
    let Some(session_token) = cookie_value(&headers, OPERATOR_SESSION_COOKIE) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation,
        );
    };
    let csrf_token =
        cookie_value(&headers, OPERATOR_CSRF_COOKIE).expect("operator CSRF cookie was validated");
    let started = std::time::Instant::now();
    if let Err(status) = IdentityAccessClient::validate_operator_session(
        &state.identity_client,
        session_token.clone(),
        csrf_token,
        &correlation,
    )
    .await
    {
        return grpc_error(
            status,
            "validate_operator_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        );
    }
    match IdentityAccessClient::operator_logout(&state.identity_client, session_token, &correlation)
        .await
    {
        Ok(_) => clear_session(
            response_no_content(),
            OPERATOR_SESSION_COOKIE,
            OPERATOR_CSRF_COOKIE,
        ),
        Err(status) => grpc_error(
            status,
            "revoke_operator_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}

#[utoipa::path(delete, path = "/v1/identity/session", responses((status = 204), (status = 401, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub async fn logout(State(state): State<std::sync::Arc<AppState>>, headers: HeaderMap) -> Response {
    let correlation = correlation_id(&headers);
    let token = match validate_mutation_session(&state, &headers, &correlation).await {
        Ok(token) => token,
        Err(response) => return *response,
    };
    let started = std::time::Instant::now();
    match IdentityAccessClient::logout(&state.identity_client, token, &correlation).await {
        Ok(_) => clear_session(response_no_content(), SESSION_COOKIE, CSRF_COOKIE),
        Err(status) => grpc_error(
            status,
            "revoke_session",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}
