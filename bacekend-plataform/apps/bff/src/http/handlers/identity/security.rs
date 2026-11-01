use super::*;

pub(crate) fn valid_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin == format!("https://{host}")
        || (is_loopback_authority(host) && origin == format!("http://{host}"))
}

pub(super) fn is_loopback_authority(host: &str) -> bool {
    let Ok(authority) = host.parse::<axum::http::uri::Authority>() else {
        return false;
    };
    matches!(
        authority.host(),
        "localhost" | "127.0.0.1" | "[::1]" | "::1"
    )
}

pub(crate) fn valid_csrf(headers: &HeaderMap, csrf_cookie_name: &str) -> bool {
    let Some(cookie) = cookie_value(headers, csrf_cookie_name) else {
        return false;
    };
    let Some(header) = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    cookie.as_bytes().ct_eq(header.as_bytes()).into()
}

pub(super) async fn validate_mutation_session(
    state: &AppState,
    headers: &HeaderMap,
    correlation: &str,
) -> Result<String, Box<Response>> {
    if !valid_origin(headers) || !valid_csrf(headers, CSRF_COOKIE) {
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "csrf_validation_failed",
            "authorization",
            false,
            correlation.to_owned(),
        )));
    }
    let Some(session_token) = cookie_value(headers, SESSION_COOKIE) else {
        return Err(Box::new(error(
            StatusCode::UNAUTHORIZED,
            "authentication_failed",
            "authentication",
            false,
            correlation.to_owned(),
        )));
    };
    let Some(csrf_token) = cookie_value(headers, CSRF_COOKIE) else {
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "csrf_validation_failed",
            "authorization",
            false,
            correlation.to_owned(),
        )));
    };
    let started = std::time::Instant::now();
    match IdentityAccessClient::validate_session(
        &state.identity_client,
        session_token.clone(),
        csrf_token,
        correlation,
    )
    .await
    {
        Ok(_) => Ok(session_token),
        Err(status) => Err(Box::new(grpc_error(
            status,
            "validate_tenant_session",
            correlation,
            started.elapsed().as_millis() as u64,
        ))),
    }
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name && !value.is_empty() && value.len() <= 256).then(|| value.to_owned())
        })
}

pub(super) fn append_cookie(
    response: &mut Response,
    cookie: String,
) -> Result<(), axum::http::header::InvalidHeaderValue> {
    response
        .headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    Ok(())
}

pub(super) fn response_no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

pub(super) fn clear_session(mut response: Response, session_cookie: &str, csrf_cookie: &str) -> Response {
    let session_cookie_value =
        format!("{session_cookie}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0");
    let csrf_cookie_value = format!("{csrf_cookie}=; Path=/; Secure; SameSite=Lax; Max-Age=0");
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&session_cookie_value)
            .expect("known session cookie name is a valid header value"),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&csrf_cookie_value)
            .expect("known CSRF cookie name is a valid header value"),
    );
    response
}
