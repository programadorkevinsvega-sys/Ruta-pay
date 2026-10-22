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
