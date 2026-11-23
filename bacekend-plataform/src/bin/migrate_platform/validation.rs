use url::Url;

pub(super) fn validate_database_targets(
    migration_database_url: &str,
    runtime_database_url: &str,
    runtime_role: &str,
) -> Result<(), &'static str> {
    let migration_url = Url::parse(migration_database_url)
        .map_err(|_| "platform_migration_database_url_invalid")?;
    let runtime_url =
        Url::parse(runtime_database_url).map_err(|_| "platform_database_url_invalid")?;
    if !matches!(migration_url.scheme(), "postgres" | "postgresql")
        || !matches!(runtime_url.scheme(), "postgres" | "postgresql")
        || migration_url.path() != runtime_url.path()
        || migration_url.username().is_empty()
        || runtime_url.username().is_empty()
        || migration_url.username() == runtime_url.username()
        || runtime_url.username() != runtime_role
        || !valid_role_identifier(runtime_role)
    {
        return Err("platform_migration_database_target_invalid");
    }
    Ok(())
}

pub(super) fn valid_role_identifier(role: &str) -> bool {
    let mut characters = role.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && role.len() <= 63
}

pub(super) fn validate_production_database_tls(database_url: &str) -> bool {
    Url::parse(database_url).is_ok_and(|url| {
        let parameters = url.query_pairs().collect::<Vec<_>>();
        ["sslmode", "sslrootcert", "sslcert", "sslkey"]
            .iter()
            .all(|required| {
                parameters
                    .iter()
                    .any(|(key, value)| key == required && !value.is_empty())
            })
            && parameters
                .iter()
                .any(|(key, value)| key == "sslmode" && value == "verify-full")
    })
}

pub(super) fn configure_database_tls_mode(
    database_url: &str,
    require_tls: bool,
) -> Result<String, &'static str> {
    if require_tls {
        return validate_production_database_tls(database_url)
            .then(|| database_url.to_owned())
            .ok_or("platform_migration_tls_configuration_invalid");
    }

    payvia_platform_core::config::app_config::plaintext_database_url(database_url)
        .map_err(|_| "platform_migration_database_url_invalid")
}

#[cfg(test)]
mod tests {
    use super::{
        configure_database_tls_mode, valid_role_identifier, validate_database_targets,
        validate_production_database_tls,
    };
    use url::Url;

    #[test]
    fn migration_requires_a_distinct_runtime_role_and_matching_database() {
        assert_eq!(
            validate_database_targets(
                "postgres://migrator:secret@db.internal/payvia",
                "postgres://runtime:secret@proxy.internal/payvia",
                "runtime"
            ),
            Ok(())
        );
        assert_eq!(
            validate_database_targets(
                "postgres://runtime:secret@db.internal/payvia",
                "postgres://runtime:secret@db.internal/payvia",
                "runtime"
            ),
            Err("platform_migration_database_target_invalid")
        );
        assert_eq!(
            validate_database_targets(
                "postgres://migrator:secret@db.internal/platform",
                "postgres://runtime:secret@db.internal/payvia",
                "runtime"
            ),
            Err("platform_migration_database_target_invalid")
        );
    }

    #[test]
    fn runtime_role_name_is_a_safe_postgresql_identifier() {
        assert!(valid_role_identifier("platform_runtime"));
        assert!(!valid_role_identifier("runtime;DROP ROLE admin"));
        assert!(!valid_role_identifier(""));
    }

    #[test]
    fn production_database_urls_require_full_server_and_client_verification() {
        assert!(validate_production_database_tls(
            "postgres://migration:password@db.internal/payvia?sslmode=verify-full&sslrootcert=/etc/ca.pem&sslcert=/etc/migration.crt&sslkey=/etc/migration.key"
        ));
        assert!(!validate_production_database_tls(
            "postgres://migration:password@db.internal/payvia?sslmode=require&sslrootcert=/etc/ca.pem&sslcert=/etc/migration.crt&sslkey=/etc/migration.key"
        ));
        assert!(!validate_production_database_tls(
            "postgres://migration:password@db.internal/payvia?sslmode=verify-full&sslrootcert=/etc/ca.pem"
        ));
    }

    #[test]
    fn development_database_urls_disable_tls_and_remove_vault_rendered_certificates() {
        let configured = "postgres://runtime:password@db.internal/payvia?sslmode=verify-full&sslrootcert=/run/vault/ca.pem&sslcert=/run/vault/client.crt&sslkey=/run/vault/client.key&application_name=platform";
        let plain = configure_database_tls_mode(configured, false)
            .expect("development mode accepts a valid database URL");
        let parsed = Url::parse(&plain).expect("normalized URL parses");
        assert_eq!(
            parsed.query_pairs().find(|(key, _)| key == "sslmode"),
            Some(("sslmode".into(), "disable".into()))
        );
        assert!(!parsed.query().unwrap_or_default().contains("sslcert"));
        assert!(
            parsed
                .query_pairs()
                .any(|(key, value)| key == "application_name" && value == "platform")
        );
    }

    #[test]
    fn tls_enabled_database_urls_require_verified_server_and_client_certificates() {
        assert!(
            configure_database_tls_mode(
                "postgres://runtime:password@db.internal/payvia?sslmode=disable",
                true
            )
            .is_err()
        );
    }
}
