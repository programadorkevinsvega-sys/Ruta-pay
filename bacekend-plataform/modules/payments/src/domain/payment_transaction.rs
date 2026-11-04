use uuid::Uuid;

use crate::domain::{payment_attempt::PaymentAttempt, payment_intent::PaymentIntent};

use super::payment_lifecycle::{
    LifecycleModelError, PaymentEnvironment, PaymentTransactionStatus, TransitionAuthority,
    TransitionEffect, TransitionGuardError, validate_opaque_reference, validate_provider_code,
};

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
        authority: TransitionAuthority,
    ) -> Result<TransitionEffect, TransitionGuardError> {
        let effect = self.status.transition_to(next, authority)?;
        if effect == TransitionEffect::Apply {
            self.status = next;
        }
        Ok(effect)
    }

    pub fn new(
        id: Uuid,
        intent: &PaymentIntent,
        attempt: &PaymentAttempt,
        provider_transaction_reference: String,
    ) -> Result<Self, LifecycleModelError> {
        if id.is_nil() || intent.id.is_nil() || intent.tenant_id.is_nil() {
            return Err(LifecycleModelError::IdentifierInvalid);
        }
        if attempt.payment_intent_id() != intent.id
            || attempt.tenant_id() != intent.tenant_id
            || attempt.environment().as_str() != intent.environment
        {
            return Err(LifecycleModelError::OwnershipMismatch);
        }
        if attempt.status() == super::payment_lifecycle::PaymentAttemptStatus::Created {
            return Err(LifecycleModelError::AttemptNotSubmitted);
        }
        validate_provider_code(attempt.provider_code())?;
        validate_opaque_reference(attempt.provider_account_reference())?;
        validate_opaque_reference(&provider_transaction_reference)?;
        if intent.amount_minor <= 0 {
            return Err(LifecycleModelError::AmountInvalid);
        }
        if intent.currency.len() != 3
            || !intent
                .currency
                .bytes()
                .all(|byte| byte.is_ascii_uppercase())
        {
            return Err(LifecycleModelError::CurrencyInvalid);
        }
        let environment = PaymentEnvironment::parse(&intent.environment)?;
        Ok(Self {
            id,
            tenant_id: intent.tenant_id,
            payment_intent_id: intent.id,
            payment_attempt_id: attempt.id(),
            environment,
            provider_code: attempt.provider_code().to_owned(),
            provider_account_reference: attempt.provider_account_reference().to_owned(),
            provider_transaction_reference,
            amount_minor: intent.amount_minor,
            currency: intent.currency.clone(),
            status: PaymentTransactionStatus::Pending,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        payment_attempt::PaymentAttempt,
        payment_intent::{PaymentIntent, PaymentMethodType},
    };

    fn intent() -> PaymentIntent {
        PaymentIntent {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            environment: "live".to_owned(),
            amount_minor: 100,
            currency: "COP".to_owned(),
            payment_method_type: PaymentMethodType::Card,
            merchant_reference: None,
            status: "created".to_owned(),
            created_at: chrono::Utc::now(),
        }
    }

    fn submitted_attempt(intent: &PaymentIntent) -> PaymentAttempt {
        let mut attempt = PaymentAttempt::new(
            Uuid::new_v4(),
            intent,
            1,
            "wompi".to_owned(),
            "account_01".to_owned(),
        )
        .expect("attempt identity is valid");
        attempt
            .apply_status_transition(
                super::super::payment_lifecycle::PaymentAttemptStatus::Submitted,
                TransitionAuthority::OperationSubmitted,
            )
            .expect("operation submission transition is valid");
        attempt
    }

    fn transaction(intent: &PaymentIntent, attempt: &PaymentAttempt) -> PaymentTransaction {
        PaymentTransaction::new(Uuid::new_v4(), intent, attempt, "txn_01".to_owned())
            .expect("transaction belongs to its submitted attempt")
    }

    #[test]
    fn transaction_status_is_guarded_and_terminal_state_is_immutable() {
        let intent = intent();
        let attempt = submitted_attempt(&intent);
        let mut transaction = transaction(&intent, &attempt);

        assert_eq!(
            transaction.apply_status_transition(
                PaymentTransactionStatus::Authorized,
                TransitionAuthority::ProviderEvent
            ),
            Ok(TransitionEffect::Apply)
        );
        assert_eq!(transaction.status(), PaymentTransactionStatus::Authorized);
        assert_eq!(
            transaction.apply_status_transition(
                PaymentTransactionStatus::Succeeded,
                TransitionAuthority::ProviderEvent
            ),
            Ok(TransitionEffect::Apply)
        );
        assert_eq!(
            transaction.apply_status_transition(
                PaymentTransactionStatus::Failed,
                TransitionAuthority::ProviderEvent
            ),
            Err(TransitionGuardError::ReconciliationRequired)
        );
        assert_eq!(transaction.status(), PaymentTransactionStatus::Succeeded);
    }

    #[test]
    fn transaction_rejects_a_non_submitted_attempt() {
        let intent = intent();
        let attempt = PaymentAttempt::new(
            Uuid::new_v4(),
            &intent,
            1,
            "wompi".to_owned(),
            "account_01".to_owned(),
        )
        .expect("attempt identity is valid");
        assert_eq!(
            PaymentTransaction::new(Uuid::new_v4(), &intent, &attempt, "txn_01".to_owned()),
            Err(LifecycleModelError::AttemptNotSubmitted)
        );
    }

    #[test]
    fn transaction_rejects_attempt_owned_by_another_intent() {
        let primary_intent = intent();
        let unrelated_intent = intent();
        let attempt = submitted_attempt(&unrelated_intent);
        assert_eq!(
            PaymentTransaction::new(
                Uuid::new_v4(),
                &primary_intent,
                &attempt,
                "txn_01".to_owned()
            ),
            Err(LifecycleModelError::OwnershipMismatch)
        );
    }

    #[test]
    fn transaction_rejects_invalid_provider_reference_and_money() {
        let intent = intent();
        let attempt = submitted_attempt(&intent);
        assert_eq!(
            PaymentTransaction::new(
                Uuid::new_v4(),
                &intent,
                &attempt,
                "bad\nreference".to_owned()
            ),
            Err(LifecycleModelError::ProviderReferenceInvalid)
        );
        let mut invalid_intent = intent.clone();
        invalid_intent.amount_minor = 0;
        assert_eq!(
            PaymentTransaction::new(
                Uuid::new_v4(),
                &invalid_intent,
                &attempt,
                "txn_01".to_owned()
            ),
            Err(LifecycleModelError::AmountInvalid)
        );
    }

    #[test]
    fn transaction_starts_pending_and_inherits_intent_money_and_attempt_identity() {
        let intent = intent();
        let attempt = submitted_attempt(&intent);
        let transaction = transaction(&intent, &attempt);
        assert_eq!(transaction.tenant_id(), intent.tenant_id);
        assert_eq!(transaction.payment_intent_id(), intent.id);
        assert_eq!(transaction.payment_attempt_id(), attempt.id());
        assert_eq!(transaction.amount_minor(), intent.amount_minor);
        assert_eq!(transaction.currency(), intent.currency);
        assert_eq!(transaction.provider_code(), attempt.provider_code());
        assert_eq!(transaction.status(), PaymentTransactionStatus::Pending);
    }
}
