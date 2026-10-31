use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("correlation identifier is invalid")]
pub struct CorrelationIdError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrelationId(String);

impl CorrelationId {
    pub fn parse(value: String) -> Result<Self, CorrelationIdError> {
        let valid = !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte));
        valid.then_some(Self(value)).ok_or(CorrelationIdError)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{CorrelationId, CorrelationIdError};

    #[test]
    fn accepts_minimum_and_maximum_length_correlation_ids() {
        assert_eq!(CorrelationId::parse("a".to_owned()).unwrap().as_str(), "a");
        let maximum = "a".repeat(128);
        assert_eq!(
            CorrelationId::parse(maximum.clone()).unwrap().as_str(),
            maximum
        );
    }

    #[test]
    fn rejects_empty_and_overlong_correlation_ids() {
        assert_eq!(CorrelationId::parse(String::new()), Err(CorrelationIdError));
        assert_eq!(
            CorrelationId::parse("a".repeat(129)),
            Err(CorrelationIdError)
        );
    }

    #[test]
    fn rejects_control_spaces_and_untrusted_delimiters() {
        for invalid in ["has space", "line\\nbreak", "value/slash", "value\\\"quote"] {
            assert_eq!(
                CorrelationId::parse(invalid.to_owned()),
                Err(CorrelationIdError),
                "must reject {invalid:?}"
            );
        }
    }

    #[test]
    fn accepts_only_the_documented_ascii_identifier_alphabet() {
        for valid in ["request-123", "trace_id.v2", "A1_b-2.c"] {
            assert_eq!(
                CorrelationId::parse(valid.to_owned()).unwrap().as_str(),
                valid
            );
        }
        for invalid in ["áccent", "emoji-🙂", "tab\\tvalue", "semi;colon"] {
            assert_eq!(
                CorrelationId::parse(invalid.to_owned()),
                Err(CorrelationIdError)
            );
        }
    }
}
