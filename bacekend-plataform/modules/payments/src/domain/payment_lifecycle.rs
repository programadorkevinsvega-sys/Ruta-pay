use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentEnvironment {
    Test,
    Live,
}

impl PaymentEnvironment {
    pub fn parse(value: &str) -> Result<Self, LifecycleModelError> {
        match value {
            "test" => Ok(Self::Test),
            "live" => Ok(Self::Live),
            _ => Err(LifecycleModelError::EnvironmentInvalid),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Live => "live",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentIntentStatus {
    Created,
    Processing,
    RequiresAction,
    Authorized,
    Succeeded,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentAttemptStatus {
    Created,
    Submitted,
    Processing,
    RequiresAction,
    Authorized,
    Succeeded,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentTransactionStatus {
    Pending,
    Authorized,
    Succeeded,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizedProviderPaymentStatus {
    Processing,
    RequiresAction,
    Authorized,
    Succeeded,
    Failed,
}

impl NormalizedProviderPaymentStatus {
    pub fn intent_status(self) -> Option<PaymentIntentStatus> {
        match self {
            Self::Processing => Some(PaymentIntentStatus::Processing),
            Self::RequiresAction => Some(PaymentIntentStatus::RequiresAction),
            Self::Authorized => Some(PaymentIntentStatus::Authorized),
            Self::Succeeded => Some(PaymentIntentStatus::Succeeded),
            Self::Failed => None,
        }
    }

    pub fn attempt_status(self) -> PaymentAttemptStatus {
        match self {
            Self::Processing => PaymentAttemptStatus::Processing,
            Self::RequiresAction => PaymentAttemptStatus::RequiresAction,
            Self::Authorized => PaymentAttemptStatus::Authorized,
            Self::Succeeded => PaymentAttemptStatus::Succeeded,
            Self::Failed => PaymentAttemptStatus::Failed,
        }
    }

    pub fn transaction_status(self) -> PaymentTransactionStatus {
        match self {
            Self::Processing | Self::RequiresAction => PaymentTransactionStatus::Pending,
            Self::Authorized => PaymentTransactionStatus::Authorized,
            Self::Succeeded => PaymentTransactionStatus::Succeeded,
            Self::Failed => PaymentTransactionStatus::Failed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionAuthority {
    OperationSubmitted,
    ProviderEvent,
    OperationTimeout,
    Reconciliation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionEffect {
    Apply,
    Duplicate,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionGuardError {
    #[error("payment lifecycle transition is not allowed")]
    TransitionNotAllowed,
    #[error("the event conflicts with terminal state and requires reconciliation")]
    ReconciliationRequired,
    #[error("provider event timestamp is not strictly newer than the applied event")]
    EventIsNotNewer,
}

trait LifecycleState: Copy + Eq {
    fn can_transition_to(self, next: Self) -> bool;
    fn is_terminal(self) -> bool;
    fn is_outcome_unknown(self) -> bool;
    fn can_become_outcome_unknown(self) -> bool;
    fn requires_operation_submission(self, next: Self) -> bool;
}

fn evaluate_transition<S: LifecycleState>(
    current: S,
    next: S,
    authority: TransitionAuthority,
) -> Result<TransitionEffect, TransitionGuardError> {
    if current == next {
        return Ok(TransitionEffect::Duplicate);
    }
    if current.is_terminal() {
        return Err(TransitionGuardError::ReconciliationRequired);
    }
    if current.is_outcome_unknown() {
        return if authority == TransitionAuthority::Reconciliation && !next.is_outcome_unknown() {
            Ok(TransitionEffect::Apply)
        } else {
            Err(TransitionGuardError::ReconciliationRequired)
        };
    }
    if current.requires_operation_submission(next) {
        return if authority == TransitionAuthority::OperationSubmitted {
            Ok(TransitionEffect::Apply)
        } else {
            Err(TransitionGuardError::TransitionNotAllowed)
        };
    }
    if next.is_outcome_unknown() {
        return if authority == TransitionAuthority::OperationTimeout
            && current.can_become_outcome_unknown()
        {
            Ok(TransitionEffect::Apply)
        } else {
            Err(TransitionGuardError::TransitionNotAllowed)
        };
    }
    if current.can_transition_to(next) {
        Ok(TransitionEffect::Apply)
    } else {
        Err(TransitionGuardError::TransitionNotAllowed)
    }
}

impl PaymentIntentStatus {
    pub fn transition_to(
        self,
        next: Self,
        authority: TransitionAuthority,
    ) -> Result<TransitionEffect, TransitionGuardError> {
        evaluate_transition(self, next, authority)
    }
}

impl LifecycleState for PaymentIntentStatus {
    fn can_transition_to(self, next: Self) -> bool {
        use PaymentIntentStatus::*;
        matches!(
            (self, next),
            (
                Created,
                Processing | RequiresAction | Authorized | Succeeded
            ) | (
                Processing,
                RequiresAction | Authorized | Succeeded
            ) | (
                RequiresAction,
                Processing | Authorized | Succeeded
            ) | (Authorized, Succeeded)
        )
    }

    fn is_terminal(self) -> bool {
        self == Self::Succeeded
    }

    fn is_outcome_unknown(self) -> bool {
        self == Self::OutcomeUnknown
    }

    fn can_become_outcome_unknown(self) -> bool {
        matches!(self, Self::Processing | Self::RequiresAction)
    }

    fn requires_operation_submission(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Created, Self::Processing) | (Self::RequiresAction, Self::Processing)
        )
    }
}

impl PaymentAttemptStatus {
    pub fn transition_to(
        self,
        next: Self,
        authority: TransitionAuthority,
    ) -> Result<TransitionEffect, TransitionGuardError> {
        evaluate_transition(self, next, authority)
    }
}

impl LifecycleState for PaymentAttemptStatus {
    fn can_transition_to(self, next: Self) -> bool {
        use PaymentAttemptStatus::*;
        matches!(
            (self, next),
            (Created, Submitted)
                | (
                    Submitted,
                    Processing | RequiresAction | Authorized | Succeeded | Failed
                )
                | (
                    Processing,
                    RequiresAction | Authorized | Succeeded | Failed
                )
                | (
                    RequiresAction,
                    Processing | Authorized | Succeeded | Failed
                )
                | (Authorized, Succeeded)
        )
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }

    fn is_outcome_unknown(self) -> bool {
        self == Self::OutcomeUnknown
    }

    fn can_become_outcome_unknown(self) -> bool {
        matches!(
            self,
            Self::Submitted | Self::Processing | Self::RequiresAction
        )
    }

    fn requires_operation_submission(self, next: Self) -> bool {
        self == Self::Created && next == Self::Submitted
    }
}

impl PaymentTransactionStatus {
    pub fn transition_to(
        self,
        next: Self,
        authority: TransitionAuthority,
    ) -> Result<TransitionEffect, TransitionGuardError> {
        evaluate_transition(self, next, authority)
    }
}

impl LifecycleState for PaymentTransactionStatus {
    fn can_transition_to(self, next: Self) -> bool {
        use PaymentTransactionStatus::*;
        matches!(
            (self, next),
            (Pending, Authorized | Succeeded | Failed)
                | (Authorized, Succeeded)
        )
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }

    fn is_outcome_unknown(self) -> bool {
        self == Self::OutcomeUnknown
    }

    fn can_become_outcome_unknown(self) -> bool {
        self == Self::Pending
    }

    fn requires_operation_submission(self, _next: Self) -> bool {
        false
    }
}

pub fn ensure_provider_event_is_newer(
    previously_applied_at: Option<DateTime<Utc>>,
    incoming_at: DateTime<Utc>,
) -> Result<(), TransitionGuardError> {
    if previously_applied_at.is_some_and(|previous| incoming_at <= previous) {
        return Err(TransitionGuardError::EventIsNotNewer);
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LifecycleModelError {
    #[error("payment identifier is invalid")]
    IdentifierInvalid,
    #[error("payment attempt must be submitted before a transaction exists")]
    AttemptNotSubmitted,
    #[error("payment lifecycle ownership relationship is invalid")]
    OwnershipMismatch,
    #[error("payment environment is invalid")]
    EnvironmentInvalid,
    #[error("attempt number must be positive")]
    AttemptNumberInvalid,
    #[error("provider code is invalid")]
    ProviderCodeInvalid,
    #[error("provider reference is invalid")]
    ProviderReferenceInvalid,
    #[error("payment amount is invalid")]
    AmountInvalid,
    #[error("currency must be a three-letter uppercase code")]
    CurrencyInvalid,
}

pub(super) fn validate_provider_code(value: &str) -> Result<(), LifecycleModelError> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(LifecycleModelError::ProviderCodeInvalid);
    }
    Ok(())
}

pub(super) fn validate_opaque_reference(value: &str) -> Result<(), LifecycleModelError> {
    if value.is_empty() || value.len() > 255 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(LifecycleModelError::ProviderReferenceInvalid);
    }
    Ok(())
}

#[cfg(test)]
#[path = "payment_lifecycle_tests.rs"]
mod tests;
