use uuid::Uuid;

use super::payment_lifecycle::{
    LifecycleModelError, PaymentAttemptStatus, PaymentEnvironment, validate_opaque_reference,
    validate_provider_code,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentAttempt {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub payment_intent_id: Uuid,
    pub environment: PaymentEnvironment,
    pub attempt_number: u32,
    pub provider_code: String,
    pub provider_account_reference: String,
    pub status: PaymentAttemptStatus,
}

impl PaymentAttempt {
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
    fn attempt_requires_positive_sequence_and_normalized_provider_code() {
        let tenant_id = Uuid::new_v4();
        let intent_id = Uuid::new_v4();
        assert!(PaymentAttempt::new(
            Uuid::new_v4(),
            tenant_id,
            intent_id,
            PaymentEnvironment::Test,
            0,
            "wompi".to_owned(),
            "account_01".to_owned()
        )
        .is_err());
        assert!(PaymentAttempt::new(
            Uuid::new_v4(),
            tenant_id,
            intent_id,
            PaymentEnvironment::Test,
            1,
            "Wompi".to_owned(),
            "account_01".to_owned()
        )
        .is_err());
    }
}
