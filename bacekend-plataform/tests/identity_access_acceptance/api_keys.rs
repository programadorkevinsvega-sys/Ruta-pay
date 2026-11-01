mod validation;
mod outbox_failure;
mod create_and_authorize;
mod revoke;

use crate::identity_access_acceptance_context::AcceptanceContext;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
    validation::verify(ctx).await;
    outbox_failure::verify(ctx).await;
    create_and_authorize::verify(ctx).await;
    revoke::verify(ctx).await;
}
