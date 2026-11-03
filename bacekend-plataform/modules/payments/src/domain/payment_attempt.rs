use uuid::Uuid;

use super::payment_lifecycle::{
    LifecycleModelError, PaymentAttemptStatus, PaymentEnvironment, validate_opaque_reference,
    validate_provider_code,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentAttempt {
    id: Uuid,
    tenant_id: Uuid,
    payment_intent_id: Uuid,
    environment: PaymentEnvironment,
    attempt_number: u32,
    provider_code: String,
    provider_account_reference: String,
    status: PaymentAttemptStatus,
}

impl PaymentAttempt {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    pub fn payment_intent_id(&self) -> Uuid {
        self.payment_intent_id
    }

    pub fn environment(&self) -> PaymentEnvironment {
        self.environment
    }

    pub fn attempt_number(&self) -> u32 {
        self.attempt_number
    }

    pub fn provider_code(&self) -> &str {
        &self.provider_code
    }

    pub fn provider_account_reference(&self) -> &str {
        &self.provider_account_reference
    }

    pub fn status(&self) -> PaymentAttemptStatus {
        self.status
    }

    pub fn apply_status_transition(
        &mut self,
        next: PaymentAttemptStatus,
        authority: super::payment_lifecycle::TransitionAuthority,
    ) -> Result<super::payment_lifecycle::TransitionEffect, super::payment_lifecycle::TransitionGuardError> {
        let effect = self.status.transition_to(next, authority)?;
        if effect == super::payment_lifecycle::TransitionEffect::Apply {
            self.status = next;
        }
        Ok(effect)
    }

    pub fn new(
        id: Uuid,
        tenant_id: Uuid,
        payment_intent_id: Uuid,
        environment: PaymentEnvironment,
        attempt_number: u32,
        provider_code: String,
        provider_account_reference: String,
    ) -> Result<Self, LifecycleModelError> {
        validate_provider_code(&provider_code)?;
        validate_opaque_reference(&provider_account_reference)?;
        if attempt_number == 0 {
            return Err(LifecycleModelError::AttemptNumberInvalid);
        }
        Ok(Self {
            id,
            tenant_id,
            payment_intent_id,
            environment,
            attempt_number,
            provider_code,
            provider_account_reference,
            status: PaymentAttemptStatus::Created,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_changes_are_applied_only_after_guard_acceptance() {
        let mut attempt = PaymentAttempt::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            PaymentEnvironment::Test,
            1,
            "wompi".to_owned(),
            "account_01".to_owned(),
        )
        .expect("attempt identity is valid");

        assert_eq!(
            attempt.apply_status_transition(
                PaymentAttemptStatus::Submitted,
                super::super::payment_lifecycle::TransitionAuthority::ProviderEvent
            ),
            Ok(super::super::payment_lifecycle::TransitionEffect::Apply)
        );
        assert_eq!(attempt.status(), PaymentAttemptStatus::Submitted);
        assert_eq!(
            attempt.apply_status_transition(
                PaymentAttemptStatus::Submitted,
                super::super::payment_lifecycle::TransitionAuthority::ProviderEvent
            ),
            Ok(super::super::payment_lifecycle::TransitionEffect::Duplicate)
        );
        assert_eq!(attempt.status(), PaymentAttemptStatus::Submitted);
    }

    #[test]
    fn attempt_requires_positive_sequence_and_normalized_provider_code() {
        let tenant_id = Uuid::new_v4();
        let intent_id = Uuid::new_v4();
        assert!(
            PaymentAttempt::new(
                Uuid::new_v4(),
                tenant_id,
                intent_id,
                PaymentEnvironment::Test,
                0,
                "wompi".to_owned(),
                "account_01".to_owned()
            )
            .is_err()
        );
        assert!(
            PaymentAttempt::new(
                Uuid::new_v4(),
                tenant_id,
                intent_id,
                PaymentEnvironment::Test,
                1,
                "Wompi".to_owned(),
                "account_01".to_owned()
            )
            .is_err()
        );
    }
}
