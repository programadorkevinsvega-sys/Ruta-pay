use uuid::Uuid;

use super::payment_lifecycle::{
    LifecycleModelError, PaymentEnvironment, PaymentTransactionStatus, validate_opaque_reference,
    validate_provider_code,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentTransactionIdentity {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub payment_intent_id: Uuid,
    pub payment_attempt_id: Uuid,
    pub environment: PaymentEnvironment,
    pub provider_code: String,
    pub provider_account_reference: String,
    pub provider_transaction_reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentTransaction {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub payment_intent_id: Uuid,
    pub payment_attempt_id: Uuid,
    pub environment: PaymentEnvironment,
    pub provider_code: String,
    pub provider_account_reference: String,
    pub provider_transaction_reference: String,
    pub amount_minor: i64,
    pub currency: String,
    pub status: PaymentTransactionStatus,
}

impl PaymentTransaction {
    pub fn new(
        identity: PaymentTransactionIdentity,
        amount_minor: i64,
        currency: String,
    ) -> Result<Self, LifecycleModelError> {
        validate_provider_code(&identity.provider_code)?;
        validate_opaque_reference(&identity.provider_account_reference)?;
        validate_opaque_reference(&identity.provider_transaction_reference)?;
        if amount_minor <= 0 {
            return Err(LifecycleModelError::AmountInvalid);
        }
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(LifecycleModelError::CurrencyInvalid);
        }
        Ok(Self {
            id: identity.id,
            tenant_id: identity.tenant_id,
            payment_intent_id: identity.payment_intent_id,
            payment_attempt_id: identity.payment_attempt_id,
            environment: identity.environment,
            provider_code: identity.provider_code,
            provider_account_reference: identity.provider_account_reference,
            provider_transaction_reference: identity.provider_transaction_reference,
            amount_minor,
            currency,
            status: PaymentTransactionStatus::Pending,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> PaymentTransactionIdentity {
        PaymentTransactionIdentity {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            payment_intent_id: Uuid::new_v4(),
            payment_attempt_id: Uuid::new_v4(),
            environment: PaymentEnvironment::Live,
            provider_code: "wompi".to_owned(),
            provider_account_reference: "account_01".to_owned(),
            provider_transaction_reference: "txn_01".to_owned(),
        }
    }

    #[test]
    fn transaction_rejects_invalid_money() {
        assert!(PaymentTransaction::new(identity(), 0, "COP".to_owned()).is_err());
        assert!(PaymentTransaction::new(identity(), 100, "cop".to_owned()).is_err());
    }

    #[test]
    fn transaction_starts_pending_with_valid_financial_identity() {
        let transaction =
            PaymentTransaction::new(identity(), 100, "COP".to_owned()).expect("valid transaction");
        assert_eq!(transaction.amount_minor, 100);
        assert_eq!(transaction.currency, "COP");
        assert_eq!(transaction.status, PaymentTransactionStatus::Pending);
    }
}
