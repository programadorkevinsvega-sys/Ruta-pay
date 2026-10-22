use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmailAddress(String);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EmailAddressError {
    #[error("email address is invalid")]
    Invalid,
}

impl EmailAddress {
    pub fn parse(value: &str) -> Result<Self, EmailAddressError> {
        let normalized = value.trim().to_lowercase();
        let Some((local, domain)) = normalized.split_once('@') else {
            return Err(EmailAddressError::Invalid);
        };
        if normalized.len() > 254
            || local.is_empty()
            || local.len() > 64
            || domain.len() < 3
            || !domain.contains('.')
            || normalized.bytes().any(|byte| {
                !(byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-' | b'@'))
            })
            || local.starts_with('.')
            || local.ends_with('.')
            || local.contains("..")
            || domain.starts_with('.')
            || domain.ends_with('.')
            || domain.contains("..")
        {
            return Err(EmailAddressError::Invalid);
        }
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{EmailAddress, EmailAddressError};

    #[test]
    fn normalizes_email_addresses_for_unique_identity() {
        let address = EmailAddress::parse("  User+Billing@Example.COM ").expect("valid email");
        assert_eq!(address.as_str(), "user+billing@example.com");
    }

    #[test]
    fn rejects_ambiguous_or_malformed_addresses() {
        for value in [
            "",
            "user",
            "@example.com",
            "user@localhost",
            "user..name@example.com",
        ] {
            assert_eq!(EmailAddress::parse(value), Err(EmailAddressError::Invalid));
        }
    }
}
