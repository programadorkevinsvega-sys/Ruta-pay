use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PasswordError {
    #[error("password does not meet length requirements")]
    InvalidLength,
    #[error("password contains unsupported control characters")]
    InvalidCharacters,
    #[error("password is missing an uppercase letter")]
    MissingUppercase,
    #[error("password is missing a lowercase letter")]
    MissingLowercase,
    #[error("password is missing a digit")]
    MissingDigit,
    #[error("password is missing a punctuation symbol")]
    MissingSymbol,
}

pub fn validate_password(password: &str) -> Result<(), PasswordError> {
    let character_count = password.chars().count();
    if !(12..=128).contains(&character_count) {
        return Err(PasswordError::InvalidLength);
    }
    if password.chars().any(char::is_control) {
        return Err(PasswordError::InvalidCharacters);
    }
    if !password
        .chars()
        .any(|character| character.is_ascii_uppercase())
    {
        return Err(PasswordError::MissingUppercase);
    }
    if !password
        .chars()
        .any(|character| character.is_ascii_lowercase())
    {
        return Err(PasswordError::MissingLowercase);
    }
    if !password.chars().any(|character| character.is_ascii_digit()) {
        return Err(PasswordError::MissingDigit);
    }
    if !password
        .chars()
        .any(|character| character.is_ascii_punctuation())
    {
        return Err(PasswordError::MissingSymbol);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PasswordError, validate_password};

    #[test]
    fn password_policy_requires_length_and_complexity() {
        assert_eq!(
            validate_password("short"),
            Err(PasswordError::InvalidLength)
        );
        assert_eq!(
            validate_password("long-enough-password\nwith-newline"),
            Err(PasswordError::InvalidCharacters)
        );
        assert_eq!(
            validate_password("longenoughpassword"),
            Err(PasswordError::MissingUppercase)
        );
        assert_eq!(
            validate_password("LONG-ENOUGH-PASSWORD1!"),
            Err(PasswordError::MissingLowercase)
        );
        assert_eq!(
            validate_password("Long-enough-password"),
            Err(PasswordError::MissingDigit)
        );
        assert_eq!(
            validate_password("Longenoughpassword1"),
            Err(PasswordError::MissingSymbol)
        );
        assert_eq!(validate_password("Long-enough-password1!"), Ok(()));
    }
}
