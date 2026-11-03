use super::*;

fn test_config(
    grpc_addr: &str,
    metrics_addr: &str,
    metrics_token: Option<String>,
) -> AppConfig {
    AppConfig {
        database_url: env::var("PLATFORM_TEST_DATABASE_ADMIN_URL")
            .expect("isolated PostgreSQL test administrator URL is configured"),
        redis_url: env::var("PLATFORM_REDIS_URL")
            .expect("real development Redis URL is configured"),
        nats_url: env::var("PLATFORM_NATS_URL")
            .expect("real development JetStream URL is configured"),
        nats_username: env::var("NATS_USER").expect("development NATS user is configured"),
        nats_password: env::var("NATS_PASSWORD")
            .expect("development NATS password is configured"),
        http_addr: "0.0.0.0:8080".parse().expect("address"),
        grpc_addr: grpc_addr.parse().expect("address"),
        metrics_addr: metrics_addr.parse().expect("address"),
        environment: "test".to_owned(),
        version: "test-version".to_owned(),
        otlp_endpoint: None,
        sentry_dsn: None,
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
        database_url: "postgres://tester:secret@127.0.0.1/payvia".to_owned(),
        redis_url: "rediss://:secret@127.0.0.1/0".to_owned(),
        nats_url: "tls://127.0.0.1:4222".to_owned(),
        nats_username: "ci-user".to_owned(),
        nats_password: "0123456789abcdef0123456789abcdef".to_owned(),
        http_addr: "0.0.0.0:8080".parse().expect("HTTP address"),
        grpc_addr: "127.0.0.1:50051".parse().expect("gRPC address"),
        metrics_addr: "127.0.0.1:9090".parse().expect("metrics address"),
        environment: "test".to_owned(),
        version: "test-version".to_owned(),
        otlp_endpoint: None,
        sentry_dsn: None,
        metrics_token: None,
    }
}

#[test]
fn config_accepts_every_declared_environment_and_rejects_unknown_values() {
    let mut config = isolated_valid_config();
    for environment in ["development", "test", "staging", "production"] {
        config.environment = environment.to_owned();
        assert_eq!(validate_config(&config), Ok(()), "{environment}");
    }
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