use async_nats::{ConnectOptions, jetstream};
use platform_runtime::event_workers::NatsAuthentication;
use redis::{Client as RedisClient, TlsCertificates};
use std::{
    env, io,
    path::{Path, PathBuf},
};
use url::Url;

use super::common::{
    DependencyError, configuration_error, ensure_remote_test_environment, is_remote_host,
    mtls_required,
};

pub async fn connect_jetstream() -> Result<jetstream::Context, DependencyError> {
    let configured_url = required_environment("PLATFORM_NATS_URL")?;
    let configured_url =
        normalize_service_scheme(&configured_url, "tls", "nats", mtls_required()?, "NATS")?;
    let parsed_url = Url::parse(&configured_url)?;
    if !matches!(parsed_url.scheme(), "nats" | "tls")
        || !parsed_url.username().is_empty()
        || parsed_url.password().is_some()
        || parsed_url.query().is_some()
        || parsed_url.fragment().is_some()
    {
        return Err(configuration_error(
            "NATS test URL must use nats:// or tls:// without embedded credentials or query",
        )
        .into());
    }
    let authentication = nats_authentication(&configured_url)?;
    let options = nats_options(parsed_url, authentication)?;
    let client = options.connect(&configured_url).await?;
    Ok(jetstream::new(client))
}

pub fn nats_authentication(url: &str) -> Result<NatsAuthentication, DependencyError> {
    let parsed_url = Url::parse(url)?;
    let remote = is_remote_host(&parsed_url)?;
    ensure_remote_test_environment(remote)?;
    if remote && mtls_required()? {
        let username = env::var("NATS_USER").ok();
        let password = env::var("NATS_PASSWORD").ok();
        match (username, password) {
            (None, None) => return Ok(select_nats_authentication(true, None)?),
            (Some(username), Some(password))
                if !username.trim().is_empty() && !password.trim().is_empty() =>
            {
                return Ok(select_nats_authentication(
                    false,
                    Some((username, password)),
                )?);
            }
            _ => {
                return Err(
                    configuration_error("NATS credentials must be configured together").into(),
                );
            }
        }
    }
    Ok(select_nats_authentication(
        false,
        Some((
            required_environment("NATS_USER")?,
            required_environment("NATS_PASSWORD")?,
        )),
    )?)
}

fn select_nats_authentication(
    remote: bool,
    credentials: Option<(String, String)>,
) -> Result<NatsAuthentication, io::Error> {
    if remote {
        return Ok(NatsAuthentication::CertificateMapped);
    }
    let (username, password) =
        credentials.ok_or_else(|| configuration_error("local NATS credentials are required"))?;
    if username.trim().is_empty() || password.trim().is_empty() {
        return Err(configuration_error(
            "local NATS credentials must not be empty",
        ));
    }
    Ok(NatsAuthentication::UserPassword { username, password })
}

fn normalize_service_scheme(
    value: &str,
    secure_scheme: &str,
    plaintext_scheme: &str,
    tls_enabled: bool,
    dependency: &str,
) -> Result<String, DependencyError> {
    let mut url = Url::parse(value)?;
    let (source, target, expected) = if tls_enabled {
        (plaintext_scheme, secure_scheme, secure_scheme)
    } else {
        (secure_scheme, plaintext_scheme, plaintext_scheme)
    };
    if url.scheme() == source {
        url.set_scheme(target)
            .map_err(|_| configuration_error(&format!("{dependency} URL scheme is invalid")))?;
    }
    if url.scheme() != expected {
        return Err(configuration_error(&format!(
            "{dependency} URL scheme does not match PAYVIA_REQUIRE_MTLS"
        ))
        .into());
    }
    Ok(url.into())
}

pub fn redis_client() -> Result<RedisClient, DependencyError> {
    let configured_url = required_environment("PLATFORM_REDIS_URL")?;
    let tls_enabled = mtls_required()?;
    let url = normalize_service_scheme(&configured_url, "rediss", "redis", tls_enabled, "Redis")?;
    let parsed_url = Url::parse(&url)?;
    if !matches!(parsed_url.scheme(), "redis" | "rediss")
        || parsed_url.host_str().is_none()
        || parsed_url.password().is_none()
        || parsed_url.query().is_some()
        || parsed_url.fragment().is_some()
    {
        return Err(configuration_error(
            "Redis test URL must use redis:// or rediss:// without query or fragment",
        )
        .into());
    }
    let remote = is_remote_host(&parsed_url)?;
    ensure_remote_test_environment(remote)?;
    let root = if tls_enabled {
        optional_tls_path("PLATFORM_REDIS_TLS_ROOT_CA")?
    } else {
        None
    };
    let certificate = if tls_enabled {
        optional_tls_path("PLATFORM_REDIS_TLS_CLIENT_CERT")?
    } else {
        None
    };
    let private_key = if tls_enabled {
        optional_tls_path("PLATFORM_REDIS_TLS_CLIENT_KEY")?
    } else {
        None
    };
    let configured = [root.is_some(), certificate.is_some(), private_key.is_some()];

    if configured.iter().any(|is_configured| *is_configured)
        && !configured.iter().all(|is_configured| *is_configured)
    {
        return Err(configuration_error(
            "Redis mTLS requires root CA, client certificate, and client key",
        )
        .into());
    }
    validate_mtls_identity(
        "Redis",
        tls_enabled,
        configured.iter().all(|is_configured| *is_configured),
    )?;

    match (root, certificate, private_key) {
        (Some(root), Some(certificate), Some(private_key)) => {
            if parsed_url.scheme() != "rediss" {
                return Err(configuration_error("Redis mTLS requires a rediss:// endpoint").into());
            }
            let tls = TlsCertificates {
                client_tls: Some(redis::ClientTlsConfig {
                    client_cert: read_tls_file(&certificate)?,
                    client_key: read_tls_file(&private_key)?,
                }),
                root_cert: Some(read_tls_file(&root)?),
            };
            Ok(RedisClient::build_with_tls(url.as_str(), tls)?)
        }
        (None, None, None) if parsed_url.scheme() == "redis" => {
            Ok(RedisClient::open(url.as_str())?)
        }
        _ => Err(configuration_error(
            "Redis transport and client identity do not match PAYVIA_REQUIRE_MTLS",
        )
        .into()),
    }
}

fn nats_options(
    url: Url,
    authentication: NatsAuthentication,
) -> Result<ConnectOptions, DependencyError> {
    let tls_enabled = mtls_required()?;
    let root = if tls_enabled {
        optional_tls_path("NATS_TLS_ROOT_CA")?
    } else {
        None
    };
    let certificate = if tls_enabled {
        optional_tls_path("NATS_TLS_CLIENT_CERT")?
    } else {
        None
    };
    let private_key = if tls_enabled {
        optional_tls_path("NATS_TLS_CLIENT_KEY")?
    } else {
        None
    };
    let configured = [root.is_some(), certificate.is_some(), private_key.is_some()];

    if configured.iter().any(|is_configured| *is_configured)
        && !configured.iter().all(|is_configured| *is_configured)
    {
        return Err(configuration_error(
            "NATS mTLS requires root CA, client certificate, and client key",
        )
        .into());
    }
    let certificate_mapped = matches!(&authentication, NatsAuthentication::CertificateMapped);
    validate_mtls_identity(
        "NATS",
        tls_enabled || certificate_mapped,
        configured.iter().all(|is_configured| *is_configured),
    )?;

    match (root, certificate, private_key, authentication) {
        (
            Some(root),
            Some(certificate),
            Some(private_key),
            NatsAuthentication::CertificateMapped,
        ) => {
            if url.scheme() != "tls" {
                return Err(configuration_error("NATS mTLS requires a tls:// endpoint").into());
            }
            Ok(ConnectOptions::new()
                .add_root_certificates(root)
                .add_client_certificate(certificate, private_key)
                .require_tls(true))
        }
        (
            Some(root),
            Some(certificate),
            Some(private_key),
            NatsAuthentication::UserPassword { username, password },
        ) => {
            if url.scheme() != "tls" {
                return Err(configuration_error("NATS mTLS requires a tls:// endpoint").into());
            }
            Ok(ConnectOptions::with_user_and_password(username, password)
                .add_root_certificates(root)
                .add_client_certificate(certificate, private_key)
                .require_tls(true))
        }
        (None, None, None, NatsAuthentication::UserPassword { username, password })
            if url.scheme() == "nats" =>
        {
            Ok(ConnectOptions::with_user_and_password(username, password))
        }
        _ => Err(configuration_error(
            "NATS transport and client identity do not match PAYVIA_REQUIRE_MTLS",
        )
        .into()),
    }
}

fn required_environment(name: &str) -> Result<String, io::Error> {
    let value = env::var(name)
        .map_err(|_| configuration_error(&format!("required setting {name} is absent")))?;
    if value.trim().is_empty() {
        return Err(configuration_error(&format!(
            "required setting {name} is empty"
        )));
    }
    Ok(value)
}

fn optional_tls_path(name: &str) -> Result<Option<PathBuf>, io::Error> {
    match env::var_os(name).filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_file() {
                return Err(configuration_error(&format!(
                    "configured TLS file for {name} is unavailable"
                )));
            }
            Ok(Some(path))
        }
        None => Ok(None),
    }
}

fn read_tls_file(path: &Path) -> Result<Vec<u8>, io::Error> {
    std::fs::read(path)
}

fn validate_mtls_identity(
    dependency: &str,
    required: bool,
    configured: bool,
) -> Result<(), io::Error> {
    if required && !configured {
        return Err(configuration_error(&format!(
            "{dependency} mTLS requires a complete client identity"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::postgres_dependencies::validate_postgres_test_urls;
    use super::{
        is_remote_host, normalize_service_scheme, select_nats_authentication,
        validate_mtls_identity,
    };
    use platform_runtime::event_workers::NatsAuthentication;
    use url::Url;

    #[test]
    fn classifies_loopback_and_vm_hosts() {
        assert!(!is_remote_host(&Url::parse("postgresql://localhost/test").unwrap()).unwrap());
        assert!(is_remote_host(&Url::parse("postgresql://192.0.2.1/test").unwrap()).unwrap());
    }

    #[test]
    fn plaintext_postgres_removes_tls_parameters_and_preserves_other_options() {
        let configured = "postgresql://acceptance:secret@192.0.2.1/postgres?sslmode=verify-full&sslrootcert=/run/vault/ca.pem&sslcert=/run/vault/client.crt&sslkey=/run/vault/client.key&application_name=phase4";
        let normalized = Url::parse(
            &payvia_platform_core::config::app_config::plaintext_database_url(configured)
                .expect("database URL normalizes"),
        )
        .expect("normalized URL parses");
        assert_eq!(
            normalized.query_pairs().find(|(key, _)| key == "sslmode"),
            Some(("sslmode".into(), "disable".into()))
        );
        assert!(!normalized.query().unwrap_or_default().contains("sslcert"));
        assert!(
            normalized
                .query_pairs()
                .any(|(key, value)| key == "application_name" && value == "phase4")
        );
    }

    #[test]
    fn service_urls_match_the_selected_transport_mode() {
        assert_eq!(
            normalize_service_scheme(
                "rediss://redis.internal/0",
                "rediss",
                "redis",
                false,
                "Redis"
            )
            .expect("plaintext mode downgrades the scheme"),
            "redis://redis.internal/0"
        );
        assert_eq!(
            normalize_service_scheme("nats://nats.internal:4222", "tls", "nats", true, "NATS")
                .expect("TLS mode upgrades the scheme"),
            "tls://nats.internal:4222"
        );
        assert!(
            normalize_service_scheme("http://redis.internal/0", "rediss", "redis", false, "Redis")
                .is_err()
        );
    }

    #[test]
    fn local_postgres_can_keep_the_per_database_runtime_role() {
        let runtime = validate_postgres_test_urls("postgresql://admin@localhost/postgres", None)
            .expect("local test database setup remains supported");
        assert!(runtime.is_none());
    }

    #[test]
    fn postgres_runtime_identity_must_share_the_admin_endpoint() {
        let result = validate_postgres_test_urls(
            "postgresql://admin@localhost/postgres",
            Some("postgresql://runtime@192.0.2.1/postgres"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn postgres_runtime_identity_must_use_a_distinct_role() {
        let result = validate_postgres_test_urls(
            "postgresql://acceptance@localhost/postgres",
            Some("postgresql://acceptance@localhost/postgres"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn postgres_runtime_role_supports_the_documented_hyphenated_name() {
        let runtime = validate_postgres_test_urls(
            "postgresql://acceptance@localhost/postgres",
            Some("postgresql://phase4-postgres-runtime@localhost/postgres"),
        )
        .expect("documented runtime role is valid");
        assert!(runtime.is_some());
    }

    #[test]
    fn postgres_runtime_role_rejects_sql_identifier_injection() {
        let result = validate_postgres_test_urls(
            "postgresql://admin@localhost/postgres",
            Some("postgresql://runtime%22%3BDROP%20ROLE%20admin@localhost/postgres"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn requires_client_identity_only_when_mtls_is_enabled() {
        assert!(validate_mtls_identity("Redis", true, false).is_err());
        assert!(validate_mtls_identity("NATS", true, false).is_err());
        assert!(validate_mtls_identity("Redis", false, false).is_ok());
        assert!(validate_mtls_identity("NATS", false, false).is_ok());
    }

    #[test]
    fn remote_nats_uses_certificate_mapping_without_password_credentials() {
        let authentication = select_nats_authentication(true, None).expect("mapped identity");
        assert!(matches!(
            authentication,
            NatsAuthentication::CertificateMapped
        ));
        assert!(select_nats_authentication(false, None).is_err());
    }
}
