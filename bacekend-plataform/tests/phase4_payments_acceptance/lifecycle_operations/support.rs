use platform_payments::application::payment_service::{
    CreatePaymentAttemptCommand, CreatePaymentIntentCommand, UpdatePaymentIntentStatusCommand,
};
use uuid::Uuid;

pub fn intent_command(tenant_id: Uuid, actor_key_id: Uuid) -> CreatePaymentIntentCommand {
    CreatePaymentIntentCommand {
        tenant_id,
        actor_key_id,
        environment: "test".to_owned(),
        amount_minor: 1299,
        currency: "COP".to_owned(),
        payment_method_type: "card".to_owned(),
        merchant_reference: Some("phase4-lifecycle-order".to_owned()),
        idempotency_key: format!("intent-{}", Uuid::new_v4()),
        correlation_id: "phase4-lifecycle-acceptance".to_owned(),
    }
}

pub fn attempt_command(
    tenant_id: Uuid,
    actor_key_id: Uuid,
    payment_intent_id: Uuid,
    idempotency_key: &str,
) -> CreatePaymentAttemptCommand {
    CreatePaymentAttemptCommand {
        tenant_id,
        actor_key_id,
        environment: "test".to_owned(),
        payment_intent_id,
        provider_code: "wompi".to_owned(),
        provider_account_reference: "account_01".to_owned(),
        idempotency_key: idempotency_key.to_owned(),
        correlation_id: "phase4-lifecycle-acceptance".to_owned(),
    }
}

pub fn status_command(
    tenant_id: Uuid,
    actor_key_id: Uuid,
    payment_intent_id: Uuid,
    payment_attempt_id: Uuid,
    status: &str,
    expected_version: i64,
) -> UpdatePaymentIntentStatusCommand {
    UpdatePaymentIntentStatusCommand {
        tenant_id,
        actor_key_id,
        environment: "test".to_owned(),
        payment_intent_id,
        payment_attempt_id,
        status: status.to_owned(),
        expected_version,
        correlation_id: "phase4-lifecycle-acceptance".to_owned(),
    }
}
