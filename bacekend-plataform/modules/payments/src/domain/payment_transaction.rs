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
    id: Uuid,
    tenant_id: Uuid,
    payment_intent_id: Uuid,
    payment_attempt_id: Uuid,
    environment: PaymentEnvironment,
    provider_code: String,
    provider_account_reference: String,
    provider_transaction_reference: String,
    amount_minor: i64,
    currency: String,
    status: PaymentTransactionStatus,
}

impl PaymentTransaction {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    pub fn payment_intent_id(&self) -> Uuid {
        self.payment_intent_id
    }

    pub fn payment_attempt_id(&self) -> Uuid {
        self.payment_attempt_id
    }

    pub fn environment(&self) -> PaymentEnvironment {
        self.environment
    }

    pub fn provider_code(&self) -> &str {
        &self.provider_code
    }

    pub fn provider_account_reference(&self) -> &str {
        &self.provider_account_reference
    }

    pub fn provider_transaction_reference(&self) -> &str {
        &self.provider_transaction_reference
    }

    pub fn amount_minor(&self) -> i64 {
        self.amount_minor
    }

    pub fn currency(&self) -> &str {
        &self.currency
    }

    pub fn status(&self) -> PaymentTransactionStatus {
        self.status
    }

    pub fn apply_status_transition(
        &mut self,
        next: PaymentTransactionStatus,
        authority: super::payment_lifecycle::TransitionAuthority,
    ) -> Result<super::payment_lifecycle::TransitionEffect, super::payment_lifecycle::TransitionGuardError> {
        let effect = self.status.transition_to(next, authority)?;
        if effect == super::payment_lifecycle::TransitionEffect::Apply {
            self.status = next;
        }
        Ok(effect)
    }

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
    fn transaction_status_is_guarded_and_terminal_state_is_immutable() {
        let mut transaction = PaymentTransaction::new(
            identity(),
            100,
            "COP".to_owned(),
        )
        .expect("transaction identity and money are valid");

        assert_eq!(
            transaction.apply_status_transition(
                PaymentTransactionStatus::Authorized,
                super::super::payment_lifecycle::TransitionAuthority::ProviderEvent
            ),
            Ok(super::super::payment_lifecycle::TransitionEffect::Apply)
        );
        assert_eq!(transaction.status(), PaymentTransactionStatus::Authorized);
        assert_eq!(
            transaction.apply_status_transition(
                PaymentTransactionStatus::Succeeded,
                super::super::payment_lifecycle::TransitionAuthority::ProviderEvent
            ),
            Ok(super::super::payment_lifecycle::TransitionEffect::Apply)
        );
        assert_eq!(
            transaction.apply_status_transition(
                PaymentTransactionStatus::Failed,
                super::super::payment_lifecycle::TransitionAuthority::ProviderEvent
            ),
            Err(super::super::payment_lifecycle::TransitionGuardError::ReconciliationRequired)
        );
        assert_eq!(transaction.status(), PaymentTransactionStatus::Succeeded);
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
