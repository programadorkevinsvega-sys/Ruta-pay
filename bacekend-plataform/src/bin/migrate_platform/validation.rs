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

fn valid_role_identifier(role: &str) -> bool {
    let mut characters = role.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && role.len() <= 63
}

#[cfg(test)]
mod tests {
    use super::{valid_role_identifier, validate_database_targets};

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
}
