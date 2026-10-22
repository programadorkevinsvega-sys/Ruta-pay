use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use thiserror::Error;

use super::password::{PasswordError, validate_password};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PasswordHashError {
    #[error(transparent)]
    Password(#[from] PasswordError),
    #[error("password hashing failed")]
    HashingFailed,
}

pub fn hash_password(password: &str) -> Result<String, PasswordHashError> {
    validate_password(password)?;
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordHashError::HashingFailed)
}

pub fn verify_password(password: &str, encoded_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(encoded_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{hash_password, verify_password};

    #[test]
    fn stores_argon2id_hash_and_verifies_only_matching_password() {
        let hash = hash_password("Correct horse battery staple1!").expect("valid password hashes");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("Correct horse battery staple1!", &hash));
        assert!(!verify_password("Different password1!", &hash));
    }

    #[test]
    fn rejects_passwords_outside_policy_before_hashing() {
        assert!(hash_password("short").is_err());
    }
}
