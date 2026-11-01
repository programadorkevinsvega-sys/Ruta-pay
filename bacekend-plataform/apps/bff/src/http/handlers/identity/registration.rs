use super::*;

#[utoipa::path(post, path = "/v1/identity/register", request_body = Credentials, responses((status = 201, body = RegisteredUser), (status = 400, body = ErrorEnvelope), (status = 409, body = ErrorEnvelope), (status = 429, body = ErrorEnvelope), (status = 503, body = ErrorEnvelope)))]
pub async fn register(
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
    match IdentityAccessClient::register(
        &state.identity_client,
        body.email,
        body.password,
        remote.ip().to_string(),
        &correlation,
    )
    .await
    {
        Ok(response) => (
            StatusCode::CREATED,
            Json(RegisteredUser {
                user_id: response.user_id,
            }),
        )
            .into_response(),
        Err(status) => grpc_error(
            status,
            "register_tenant_user",
            &correlation,
            started.elapsed().as_millis() as u64,
        ),
    }
}
