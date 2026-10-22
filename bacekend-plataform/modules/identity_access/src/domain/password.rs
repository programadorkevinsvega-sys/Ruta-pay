use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PasswordError {
    #[error("password does not meet length requirements")]
    InvalidLength,
    #[error("password contains unsupported control characters")]
    InvalidCharacters,
}

pub fn validate_password(password: &str) -> Result<(), PasswordError> {
    if !(12..=128).contains(&password.len()) {
        return Err(PasswordError::InvalidLength);
    }
    if password.chars().any(char::is_control) {
        return Err(PasswordError::InvalidCharacters);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PasswordError, validate_password};

    #[test]
    fn password_policy_limits_length_and_control_characters() {
        assert_eq!(
            validate_password("short"),
            Err(PasswordError::InvalidLength)
        );
        assert_eq!(
            validate_password("long-enough-password\nwith-newline"),
            Err(PasswordError::InvalidCharacters)
        );
        assert_eq!(validate_password("long-enough-password"), Ok(()));
    }
}
