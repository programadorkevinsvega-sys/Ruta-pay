use super::*;

fn test_config(grpc_addr: &str, metrics_addr: &str, metrics_token: Option<String>) -> AppConfig {
    AppConfig {
        database_url: "postgres://tester:secret@127.0.0.1/payvia?sslmode=disable".to_owned(),
        redis_url: "redis://:secret@127.0.0.1/0".to_owned(),
        redis_tls_root_ca: None,
        redis_tls_client_cert: None,
        redis_tls_client_key: None,
        nats_tls_root_ca: None,
        nats_tls_client_cert: None,
        nats_tls_client_key: None,
        nats_url: "nats://127.0.0.1:4222".to_owned(),
        nats_username: "development-user".to_owned(),
        nats_password: "0123456789abcdef0123456789abcdef".to_owned(),
        http_addr: "0.0.0.0:8080".parse().expect("address"),
        grpc_addr: grpc_addr.parse().expect("address"),
        metrics_addr: metrics_addr.parse().expect("address"),
        environment: "test".to_owned(),
        require_mtls: false,
        version: "test-version".to_owned(),
        otlp_endpoint: None,
        sentry_dsn: None,
        sentry_traces_sample_rate: None,
        metrics_token,
    }
}

#[test]
fn defaults_are_valid_and_bind_metrics_locally() {
    let database_url = env::var("PLATFORM_TEST_DATABASE_ADMIN_URL")
        .expect("isolated PostgreSQL test administrator URL is configured");
    let redis_url =
        env::var("PLATFORM_REDIS_URL").expect("real development Redis URL is configured");
    assert!(validate_database_url(&database_url));
    assert!(validate_redis_url(&redis_url));
}

#[test]
fn otlp_endpoint_rejects_credentials_and_query_strings() {
    assert!(!validate_otlp_endpoint(
        "https://user:password@collector.example:4317"
    ));
    assert!(!validate_otlp_endpoint(
        "https://collector.example:4317?token=sensitive"
    ));
    assert!(validate_otlp_endpoint("https://collector.example:4317"));
}

#[test]
fn sentry_trace_sample_rate_is_bounded_and_finite() {
    assert!(valid_sentry_traces_sample_rate(0.0));
    assert!(valid_sentry_traces_sample_rate(1.0));
    assert!(!valid_sentry_traces_sample_rate(-0.1));
    assert!(!valid_sentry_traces_sample_rate(1.1));
    assert!(!valid_sentry_traces_sample_rate(f32::NAN));
    assert!(!valid_sentry_traces_sample_rate(f32::INFINITY));
}

#[test]
fn database_url_requires_postgres_credentials_and_rejects_non_postgres_schemes() {
    let database_url = env::var("PLATFORM_TEST_DATABASE_ADMIN_URL")
        .expect("isolated PostgreSQL test administrator URL is configured");
    assert!(validate_database_url(&database_url));
    assert!(!validate_database_url("redis://database.invalid/payvia"));
    assert!(!validate_database_url("postgres://db.internal/payvia"));
}

#[test]
fn redis_url_requires_password_and_supported_scheme() {
    let redis_url =
        env::var("PLATFORM_REDIS_URL").expect("real development Redis URL is configured");
    assert!(validate_redis_url(&redis_url));
    assert!(!validate_redis_url("redis://redis.internal/0"));
    assert!(!validate_redis_url("https://redis.internal/0"));
}

#[test]
fn non_loopback_grpc_listener_is_rejected_without_mtls() {
    let config = test_config("0.0.0.0:50051", "127.0.0.1:9090", None);
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "APP_MODULE_GRPC_ADDR"
        })
    );
}

#[test]
fn non_loopback_metrics_requires_a_long_bearer_token() {
    let config = test_config("127.0.0.1:50051", "0.0.0.0:9090", Some("short".to_owned()));
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "APP_METRICS_BEARER_TOKEN"
        })
    );
}

#[test]
fn invalid_bind_address_has_stable_error_code() {
    assert_eq!(
        parse_address("APP_HTTP_ADDR", "not-an-address"),
        Err(ConfigError::InvalidAddress {
            variable: "APP_HTTP_ADDR"
        })
    );
}

fn isolated_valid_config() -> AppConfig {
    AppConfig {
        database_url: "postgres://tester:secret@127.0.0.1/payvia?sslmode=disable".to_owned(),
        redis_url: "redis://:secret@127.0.0.1/0".to_owned(),
        redis_tls_root_ca: None,
        redis_tls_client_cert: None,
        redis_tls_client_key: None,
        nats_tls_root_ca: None,
        nats_tls_client_cert: None,
        nats_tls_client_key: None,
        nats_url: "nats://127.0.0.1:4222".to_owned(),
        nats_username: "ci-user".to_owned(),
        nats_password: "0123456789abcdef0123456789abcdef".to_owned(),
        http_addr: "0.0.0.0:8080".parse().expect("HTTP address"),
        grpc_addr: "127.0.0.1:50051".parse().expect("gRPC address"),
        metrics_addr: "127.0.0.1:9090".parse().expect("metrics address"),
        environment: "test".to_owned(),
        require_mtls: false,
        version: "test-version".to_owned(),
        otlp_endpoint: None,
        sentry_dsn: None,
        sentry_traces_sample_rate: None,
        metrics_token: None,
    }
}

#[test]
fn config_accepts_nonproduction_environments_and_rejects_unknown_values() {
    let mut config = isolated_valid_config();
    for environment in ["development", "test"] {
        config.environment = environment.to_owned();
        assert_eq!(validate_config(&config), Ok(()), "{environment}");
    }
    config.environment = "staging".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PAYVIA_REQUIRE_MTLS"
        })
    );
    config.environment = "production".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PAYVIA_REQUIRE_MTLS"
        })
    );
    for environment in ["", "prod", "local", "Production", "unknown"] {
        config.environment = environment.to_owned();
        assert_eq!(
            validate_config(&config),
            Err(ConfigError::InvalidValue {
                variable: "APP_ENV"
            }),
            "{environment}"
        );
    }
}

#[test]
fn production_requires_verified_database_tls_and_complete_redis_client_identity() {
    let mut config = isolated_valid_config();
    config.environment = "production".to_owned();
    config.require_mtls = true;
    config.database_url =
        "postgres://user:password@db.internal/payvia?sslmode=verify-full&sslrootcert=/etc/ca.pem&sslcert=/etc/client.crt&sslkey=/etc/client.key"
            .to_owned();
    config.redis_url = "rediss://:password@redis.internal/0".to_owned();
    config.nats_url = "tls://nats.internal:4222".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_TLS_CLIENT_CERT"
        })
    );

    config.redis_tls_root_ca = Some("/etc/redis/ca.pem".into());
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_TLS_CLIENT_CERT"
        })
    );
    config.redis_tls_client_cert = Some("/etc/redis/client.crt".into());
    config.redis_tls_client_key = Some("/etc/redis/client.key".into());
    config.redis_url = "redis://:password@redis.internal/0".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_URL"
        })
    );
    config.redis_url = "rediss://:password@redis.internal/0".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "NATS_TLS_CLIENT_CERT"
        })
    );
    config.nats_tls_root_ca = Some("/etc/nats/ca.pem".into());
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "NATS_TLS_CLIENT_CERT"
        })
    );
    config.nats_tls_client_cert = Some("/etc/nats/platform.crt".into());
    config.nats_tls_client_key = Some("/etc/nats/platform.key".into());
    assert_eq!(validate_config(&config), Ok(()));
    config.nats_username.clear();
    config.nats_password.clear();
    assert_eq!(validate_config(&config), Ok(()));
    config.nats_username = "partial-user".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PLATFORM_NATS_URL"
        })
    );
}

#[test]
fn production_database_url_requires_verify_full_and_client_credentials() {
    assert!(validate_production_database_url(
        "postgres://user:password@db.internal/payvia?sslmode=verify-full&sslrootcert=/etc/ca.pem&sslcert=/etc/client.crt&sslkey=/etc/client.key"
    ));
    assert!(!validate_production_database_url(
        "postgres://user:password@db.internal/payvia?sslmode=require&sslrootcert=/etc/ca.pem&sslcert=/etc/client.crt&sslkey=/etc/client.key"
    ));
    assert!(!validate_production_database_url(
        "postgres://user:password@db.internal/payvia?sslmode=verify-full&sslrootcert=/etc/ca.pem"
    ));
}

#[test]
fn mtls_can_be_enabled_for_staging_when_all_tls_material_is_configured() {
    let mut config = isolated_valid_config();
    config.environment = "staging".to_owned();
    config.require_mtls = true;
    config.database_url =
        "postgres://user:password@db.internal/payvia?sslmode=verify-full&sslrootcert=/etc/ca.pem&sslcert=/etc/client.crt&sslkey=/etc/client.key"
            .to_owned();
    config.redis_url = "rediss://:password@redis.internal/0".to_owned();
    config.nats_url = "tls://nats.internal:4222".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PLATFORM_REDIS_TLS_CLIENT_CERT"
        })
    );

    config.redis_tls_root_ca = Some("/etc/redis/ca.pem".into());
    config.redis_tls_client_cert = Some("/etc/redis/client.crt".into());
    config.redis_tls_client_key = Some("/etc/redis/client.key".into());
    config.nats_tls_root_ca = Some("/etc/nats/ca.pem".into());
    config.nats_tls_client_cert = Some("/etc/nats/client.crt".into());
    config.nats_tls_client_key = Some("/etc/nats/client.key".into());
    assert_eq!(validate_config(&config), Ok(()));
}

#[test]
fn plaintext_mode_strips_certificate_parameters_and_secure_url_schemes() {
    let database = plaintext_database_url(
        "postgres://tester:secret@db.internal/payvia?sslmode=verify-full&sslrootcert=/run/vault/ca.pem&sslcert=/run/vault/client.crt&sslkey=/run/vault/client.key&application_name=platform",
    )
    .expect("valid database URL normalizes");
    let database = url::Url::parse(&database).expect("normalized URL parses");
    assert_eq!(
        database.query_pairs().find(|(key, _)| key == "sslmode"),
        Some(("sslmode".into(), "disable".into()))
    );
    assert!(
        database
            .query_pairs()
            .any(|(key, value)| key == "application_name" && value == "platform")
    );
    assert!(!database.query().unwrap_or_default().contains("sslcert"));

    assert_eq!(
        plaintext_service_url(
            "rediss://:secret@redis.internal/0",
            "rediss",
            "redis",
            "PLATFORM_REDIS_URL",
        )
        .expect("Redis URL normalizes")
        .split(':')
        .next(),
        Some("redis")
    );
    assert_eq!(
        plaintext_service_url(
            "tls://nats.internal:4222",
            "tls",
            "nats",
            "PLATFORM_NATS_URL",
        )
        .expect("NATS URL normalizes")
        .split(':')
        .next(),
        Some("nats")
    );
}

#[test]
fn config_rejects_empty_nats_username_and_password_below_minimum_length() {
    let mut config = isolated_valid_config();
    config.nats_username = "   ".to_owned();
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PLATFORM_NATS_URL"
        })
    );

    config = isolated_valid_config();
    config.nats_password = "x".repeat(31);
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "PLATFORM_NATS_URL"
        })
    );

    config.nats_password = "x".repeat(32);
    assert_eq!(validate_config(&config), Ok(()));
}

#[test]
fn config_allows_local_metrics_without_token_and_requires_32_bytes_remotely() {
    let mut config = isolated_valid_config();
    assert_eq!(validate_config(&config), Ok(()));

    config.metrics_addr = "0.0.0.0:9090".parse().expect("metrics address");
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "APP_METRICS_BEARER_TOKEN"
        })
    );
    config.metrics_token = Some("x".repeat(31));
    assert_eq!(
        validate_config(&config),
        Err(ConfigError::InvalidValue {
            variable: "APP_METRICS_BEARER_TOKEN"
        })
    );
    config.metrics_token = Some("x".repeat(32));
    assert_eq!(validate_config(&config), Ok(()));
}

#[test]
fn database_url_rejects_missing_host_credentials_and_sensitive_query_parameters() {
    for value in [
        "postgres:///payvia",
        "postgres://tester@127.0.0.1/payvia",
        "postgres://:secret@127.0.0.1/payvia",
        "postgres://tester:secret@127.0.0.1/payvia?password=leaked",
        "postgres://tester:secret@127.0.0.1/payvia?sslpassword=leaked",
        "postgres://tester:secret@127.0.0.1/payvia?application_name=abcdefghijklmnopqrstuvwxyz1234567",
    ] {
        assert!(!validate_database_url(value), "{value}");
    }
}

#[test]
fn redis_url_rejects_query_fragment_missing_password_and_unsupported_scheme() {
    for value in [
        "redis://redis.internal/0",
        "http://:secret@redis.internal/0",
        "redis://:secret@redis.internal/0?token=value",
        "redis://:secret@redis.internal/0#fragment",
    ] {
        assert!(!validate_redis_url(value), "{value}");
    }
    assert!(validate_redis_url("redis://:secret@redis.internal/0"));
    assert!(validate_redis_url("rediss://:secret@redis.internal/0"));
}

#[test]
fn nats_url_rejects_embedded_credentials_query_fragment_and_unsupported_scheme() {
    for value in [
        "http://nats.internal:4222",
        "nats://user:secret@nats.internal:4222",
        "nats://nats.internal:4222?token=value",
        "nats://nats.internal:4222#fragment",
    ] {
        assert!(!validate_nats_url(value), "{value}");
    }
    assert!(validate_nats_url("nats://nats.internal:4222"));
    assert!(validate_nats_url("tls://nats.internal:4222"));
}

#[test]
fn otlp_endpoint_rejects_unsupported_scheme_credentials_query_and_fragment() {
    for value in [
        "ftp://collector.internal:4317",
        "https://user:secret@collector.internal:4317",
        "https://collector.internal:4317?token=value",
        "https://collector.internal:4317#fragment",
    ] {
        assert!(!validate_otlp_endpoint(value), "{value}");
    }
    assert!(validate_otlp_endpoint("http://collector.internal:4317"));
    assert!(validate_otlp_endpoint("https://collector.internal:4317"));
}
