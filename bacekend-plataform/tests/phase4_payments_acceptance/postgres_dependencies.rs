use super::common::{
    DependencyError, configuration_error, ensure_remote_test_environment, is_remote_host,
    mtls_required,
};
use std::path::Path;
use url::Url;

pub fn postgres_url_for_current_mode(value: &str) -> Result<String, DependencyError> {
    let url = Url::parse(value)?;
    let remote = is_remote_host(&url)?;
    ensure_remote_test_environment(remote)?;
    if remote && std::env::var_os("PLATFORM_TEST_POSTGRES_CONTAINER_ID").is_some() {
        return Err(configuration_error(
            "physical PostgreSQL restart is forbidden for remote test endpoints",
        )
        .into());
    }

    if mtls_required()? {
        validate_postgres_tls_url(&url)?;
        return Ok(value.to_owned());
    }
    if remote && url.password().is_none() {
        return Err(configuration_error(
            "plaintext remote PostgreSQL requires a database password",
        )
        .into());
    }
    Ok(payvia_platform_core::config::app_config::plaintext_database_url(value)?)
}

pub fn validate_postgres_test_urls(
    admin_value: &str,
    runtime_value: Option<&str>,
) -> Result<Option<String>, DependencyError> {
    let admin_value = postgres_url_for_current_mode(admin_value)?;
    let admin = Url::parse(&admin_value)?;
    let remote = is_remote_host(&admin)?;
    let Some(runtime_value) = runtime_value else {
        if remote {
            return Err(configuration_error(
                "remote PostgreSQL acceptance requires a separate runtime identity URL",
            )
            .into());
        }
        return Ok(None);
    };

    let runtime_value = postgres_url_for_current_mode(runtime_value)?;
    let runtime = Url::parse(&runtime_value)?;
    if !admin.host_str().is_some_and(|host| {
        runtime
            .host_str()
            .is_some_and(|runtime_host| host.eq_ignore_ascii_case(runtime_host))
    }) || admin.port().unwrap_or(5432) != runtime.port().unwrap_or(5432)
    {
        return Err(configuration_error(
            "PostgreSQL administrator and runtime URLs must target the same host and port",
        )
        .into());
    }
    if !valid_postgres_identifier(runtime.username()) || runtime.username() == admin.username() {
        return Err(configuration_error(
            "PostgreSQL runtime URL requires a distinct, valid database role",
        )
        .into());
    }

    if remote {
        if mtls_required()? {
            if runtime.password().is_some() {
                return Err(configuration_error(
                    "remote PostgreSQL runtime authentication must use its client certificate",
                )
                .into());
            }
            let parameter = |url: &Url, name: &str| {
                url.query_pairs()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.into_owned())
            };
            if parameter(&admin, "sslrootcert") != parameter(&runtime, "sslrootcert")
                || parameter(&admin, "sslcert") == parameter(&runtime, "sslcert")
                || parameter(&admin, "sslkey") == parameter(&runtime, "sslkey")
            {
                return Err(configuration_error(
                    "remote PostgreSQL runtime identity must use the shared CA and its own certificate/key",
                )
                .into());
            }
        } else if runtime.password().is_none() {
            return Err(configuration_error(
                "plaintext PostgreSQL runtime authentication requires its database password",
            )
            .into());
        }
    }

    Ok(Some(runtime_value))
}

fn validate_postgres_tls_url(url: &Url) -> Result<(), DependencyError> {
    let parameter = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if parameter("sslmode").as_deref() != Some("verify-full") {
        return Err(configuration_error("mTLS PostgreSQL URLs require sslmode=verify-full").into());
    }
    for name in ["sslrootcert", "sslcert", "sslkey"] {
        let path = parameter(name)
            .ok_or_else(|| configuration_error(&format!("mTLS PostgreSQL URLs require {name}")))?;
        if !Path::new(&path).is_file() {
            return Err(configuration_error(&format!(
                "configured PostgreSQL TLS file for {name} is unavailable"
            ))
            .into());
        }
    }
    Ok(())
}

fn valid_postgres_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    value.len() <= 63
        && (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
