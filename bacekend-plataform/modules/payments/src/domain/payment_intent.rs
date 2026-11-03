use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentMethodType {
    Card,
    Pse,
    Crypto,
}

impl PaymentMethodType {
    pub fn parse(value: &str) -> Result<Self, PaymentIntentError> {
        match value {
            "card" => Ok(Self::Card),
            "pse" => Ok(Self::Pse),
            "crypto" => Ok(Self::Crypto),
            _ => Err(PaymentIntentError::PaymentMethodInvalid),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::Pse => "pse",
            Self::Crypto => "crypto",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentIntent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub environment: String,
    pub amount_minor: i64,
    pub currency: String,
    pub payment_method_type: PaymentMethodType,
    pub merchant_reference: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PaymentIntentError {
    #[error("payment amount is invalid")]
    AmountInvalid,
    #[error("currency must be a three-letter uppercase code")]
    CurrencyInvalid,
    #[error("payment method type is invalid")]
    PaymentMethodInvalid,
    #[error("environment is invalid")]
    EnvironmentInvalid,
    #[error("idempotency key is invalid")]
    IdempotencyKeyInvalid,
    #[error("merchant reference is invalid")]
    MerchantReferenceInvalid,
    #[error("idempotency key was already used with a different request")]
    IdempotencyConflict,
    #[error("payment intent was not found")]
    NotFound,
}

pub fn validate_create_input(
    environment: &str,
    amount_minor: i64,
    currency: &str,
    payment_method_type: &str,
    merchant_reference: Option<&str>,
    idempotency_key: &str,
) -> Result<PaymentMethodType, PaymentIntentError> {
    if !matches!(environment, "test" | "live") {
        return Err(PaymentIntentError::EnvironmentInvalid);
    }
    if amount_minor <= 0 {
        return Err(PaymentIntentError::AmountInvalid);
    }
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(PaymentIntentError::CurrencyInvalid);
    }
    let method = PaymentMethodType::parse(payment_method_type)?;
    if idempotency_key.is_empty()
        || idempotency_key.len() > 128
        || !idempotency_key.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)
        })
    {
        return Err(PaymentIntentError::IdempotencyKeyInvalid);
    }
    if merchant_reference.is_some_and(|reference| {
        reference.is_empty()
            || reference.len() > 128
            || reference.chars().any(char::is_control)
    }) {
        return Err(PaymentIntentError::MerchantReferenceInvalid);
    }
    Ok(method)
}

#[cfg(test)]
mod tests {
    use super::{PaymentIntentError, PaymentMethodType, validate_create_input};

    fn valid() -> Result<PaymentMethodType, PaymentIntentError> {
        validate_create_input("test", 1, "COP", "card", Some("order_1"), "idem-1")
    }

    #[test]
    fn accepts_only_the_versioned_payment_method_union() {
        assert_eq!(valid(), Ok(PaymentMethodType::Card));
        assert_eq!(
            validate_create_input("live", 10, "USD", "pse", None, "idem-2"),
            Ok(PaymentMethodType::Pse)
        );
        assert_eq!(
            validate_create_input("test", 10, "EUR", "crypto", None, "idem-3"),
            Ok(PaymentMethodType::Crypto)
        );
        assert_eq!(
            validate_create_input("test", 10, "USD", "wallet", None, "idem-4"),
            Err(PaymentIntentError::PaymentMethodInvalid)
        );
    }

    #[test]
    fn rejects_invalid_amount_currency_environment_and_idempotency_key() {
        assert_eq!(
            validate_create_input("sandbox", 10, "COP", "card", None, "idem"),
            Err(PaymentIntentError::EnvironmentInvalid)
        );
        assert_eq!(
            validate_create_input("test", 0, "COP", "card", None, "idem"),
            Err(PaymentIntentError::AmountInvalid)
        );
        assert_eq!(
            validate_create_input("test", -1, "COP", "card", None, "idem"),
            Err(PaymentIntentError::AmountInvalid)
        );
        for currency in ["cop", "CO", "USDD", "1$P"] {
            assert_eq!(
                validate_create_input("test", 1, currency, "card", None, "idem"),
                Err(PaymentIntentError::CurrencyInvalid)
            );
        }
        for key in ["", "bad key", "line\nbreak", &"k".repeat(129)] {
            assert_eq!(
                validate_create_input("test", 1, "COP", "card", None, key),
                Err(PaymentIntentError::IdempotencyKeyInvalid)
            );
        }
    }

    #[test]
    fn validates_optional_merchant_reference_without_interpreting_its_business_meaning() {
        assert_eq!(
            validate_create_input("test", 1, "COP", "card", Some(""), "idem"),
            Err(PaymentIntentError::MerchantReferenceInvalid)
        );
        assert_eq!(
            validate_create_input("test", 1, "COP", "card", Some("bad\nreference"), "idem"),
            Err(PaymentIntentError::MerchantReferenceInvalid)
        );
        assert_eq!(
            validate_create_input("test", 1, "COP", "card", Some(&"r".repeat(129)), "idem"),
            Err(PaymentIntentError::MerchantReferenceInvalid)
        );
    }
}
