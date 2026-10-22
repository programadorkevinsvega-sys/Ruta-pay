use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiKeyEnvironment {
    Test,
    Live,
}

impl ApiKeyEnvironment {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Test => "test_",
            Self::Live => "live_",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Live => "live",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApiKeyError {
    #[error("API key name is invalid")]
    InvalidName,
    #[error("API key scopes are invalid")]
    InvalidScopes,
    #[error("operating system random source is unavailable")]
    RandomSourceUnavailable,
}

#[derive(PartialEq, Eq)]
pub struct NewApiKey {
    pub id: Uuid,
    pub name: String,
    pub environment: ApiKeyEnvironment,
    pub prefix: String,
    pub secret: String,
    pub digest: [u8; 32],
    pub scopes: Vec<String>,
}

impl fmt::Debug for NewApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewApiKey")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("environment", &self.environment)
            .field("prefix", &self.prefix)
            .field("secret", &"[REDACTED]")
            .field("digest", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .finish()
    }
}

pub fn create_api_key(
    name: &str,
    environment: ApiKeyEnvironment,
    scopes: &[String],
) -> Result<NewApiKey, ApiKeyError> {
    let normalized_name = name.trim();
    if !(2..=80).contains(&normalized_name.chars().count())
        || normalized_name.chars().any(char::is_control)
    {
        return Err(ApiKeyError::InvalidName);
    }
    if scopes.is_empty()
        || scopes.len() > 16
        || scopes.iter().any(|scope| {
            scope.is_empty()
                || scope.len() > 64
                || !scope.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b':'
                        || byte == b'.'
                        || byte == b'_'
                })
        })
    {
        return Err(ApiKeyError::InvalidScopes);
    }
    let mut random_bytes = [0u8; 32];
    getrandom::fill(&mut random_bytes).map_err(|_| ApiKeyError::RandomSourceUnavailable)?;
    let secret = format!(
        "{}{}",
        environment.prefix(),
        URL_SAFE_NO_PAD.encode(random_bytes)
    );
    let digest = Sha256::digest(secret.as_bytes()).into();
    let prefix_end = secret.len().min(environment.prefix().len() + 8);
    Ok(NewApiKey {
        id: Uuid::new_v4(),
        name: normalized_name.to_owned(),
        environment,
        prefix: secret[..prefix_end].to_owned(),
        secret,
        digest,
        scopes: scopes.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{ApiKeyEnvironment, ApiKeyError, create_api_key};

    #[test]
    fn generated_key_has_environment_prefix_and_only_digest_is_needed_for_storage() {
        let key = create_api_key(
            "production app",
            ApiKeyEnvironment::Live,
            &["payment_intents:create".to_owned()],
        )
        .expect("valid API key input");
        assert!(key.secret.starts_with("live_"));
        assert!(key.prefix.starts_with("live_"));
        assert_eq!(key.digest.len(), 32);
        assert_ne!(key.digest.as_slice(), key.secret.as_bytes());
    }

    #[test]
    fn rejects_empty_or_malformed_scopes() {
        assert_eq!(
            create_api_key("key name", ApiKeyEnvironment::Test, &[]),
            Err(ApiKeyError::InvalidScopes)
        );
    }

    #[test]
    fn debug_output_never_contains_the_one_time_secret_or_digest() {
        let key = create_api_key(
            "debug check",
            ApiKeyEnvironment::Test,
            &["payment_intents:create".to_owned()],
        )
        .expect("valid API key input");
        let diagnostic = format!("{key:?}");
        assert!(!diagnostic.contains(&key.secret));
        assert!(
            !diagnostic
                .contains(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.digest))
        );
        assert!(diagnostic.contains("[REDACTED]"));
    }
}
