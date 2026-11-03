use super::*;

#[test]
fn normalized_status_maps_without_provider_specific_codes() {
    assert_eq!(
        NormalizedProviderPaymentStatus::RequiresAction.attempt_status(),
        PaymentAttemptStatus::RequiresAction
    );
    assert_eq!(
        NormalizedProviderPaymentStatus::Processing.transaction_status(),
        PaymentTransactionStatus::Pending
    );
    assert_eq!(
        NormalizedProviderPaymentStatus::Succeeded.intent_status(),
        Some(PaymentIntentStatus::Succeeded)
    );
    assert_eq!(
        NormalizedProviderPaymentStatus::Failed.intent_status(),
        None,
        "an attempt failure does not decide the overall intent retry policy"
    );
}

#[test]
fn transitions_are_forward_only_and_terminal_conflicts_need_reconciliation() {
    assert_eq!(
        PaymentAttemptStatus::Created.transition_to(
            PaymentAttemptStatus::Submitted,
            TransitionAuthority::OperationSubmitted
        ),
        Ok(TransitionEffect::Apply)
    );
    assert_eq!(
        PaymentAttemptStatus::Authorized.transition_to(
            PaymentAttemptStatus::Processing,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::TransitionNotAllowed)
    );
    assert_eq!(
        PaymentIntentStatus::Created.transition_to(
            PaymentIntentStatus::Processing,
            TransitionAuthority::OperationSubmitted
        ),
        Ok(TransitionEffect::Apply)
    );
    assert_eq!(
        PaymentTransactionStatus::Pending.transition_to(
            PaymentTransactionStatus::Authorized,
            TransitionAuthority::ProviderEvent
        ),
        Ok(TransitionEffect::Apply)
    );
    assert_eq!(
        PaymentTransactionStatus::Failed.transition_to(
            PaymentTransactionStatus::Succeeded,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::ReconciliationRequired)
    );
    assert_eq!(
        PaymentIntentStatus::Succeeded.transition_to(
            PaymentIntentStatus::Processing,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::ReconciliationRequired)
    );
}

#[test]
fn outcome_unknown_requires_reconciliation_to_resolve() {
    assert_eq!(
        PaymentAttemptStatus::OutcomeUnknown.transition_to(
            PaymentAttemptStatus::Succeeded,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::ReconciliationRequired)
    );
    assert_eq!(
        PaymentAttemptStatus::OutcomeUnknown.transition_to(
            PaymentAttemptStatus::Succeeded,
            TransitionAuthority::Reconciliation
        ),
        Ok(TransitionEffect::Apply)
    );
}

#[test]
fn provider_event_timestamps_must_be_strictly_monotonic() {
    let applied_at = Utc::now();
    assert_eq!(
        ensure_provider_event_is_newer(Some(applied_at), applied_at),
        Err(TransitionGuardError::EventIsNotNewer)
    );
    assert_eq!(
        ensure_provider_event_is_newer(Some(applied_at), applied_at + chrono::Duration::seconds(1)),
        Ok(())
    );
}

#[test]
fn only_platform_timeout_can_enter_outcome_unknown() {
    assert_eq!(
        PaymentIntentStatus::Processing.transition_to(
            PaymentIntentStatus::OutcomeUnknown,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::TransitionNotAllowed)
    );
    assert_eq!(
        PaymentAttemptStatus::Processing.transition_to(
            PaymentAttemptStatus::OutcomeUnknown,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::TransitionNotAllowed)
    );
    assert_eq!(
        PaymentTransactionStatus::Pending.transition_to(
            PaymentTransactionStatus::OutcomeUnknown,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::TransitionNotAllowed)
    );
    assert_eq!(
        PaymentAttemptStatus::Processing.transition_to(
            PaymentAttemptStatus::OutcomeUnknown,
            TransitionAuthority::OperationTimeout
        ),
        Ok(TransitionEffect::Apply)
    );
    assert_eq!(
        PaymentAttemptStatus::Created.transition_to(
            PaymentAttemptStatus::OutcomeUnknown,
            TransitionAuthority::OperationTimeout
        ),
        Err(TransitionGuardError::TransitionNotAllowed)
    );
    assert_eq!(
        PaymentAttemptStatus::Submitted.transition_to(
            PaymentAttemptStatus::OutcomeUnknown,
            TransitionAuthority::OperationTimeout
        ),
        Ok(TransitionEffect::Apply)
    );
    assert_eq!(
        PaymentIntentStatus::Created.transition_to(
            PaymentIntentStatus::Processing,
            TransitionAuthority::ProviderEvent
        ),
        Err(TransitionGuardError::TransitionNotAllowed)
    );
}
