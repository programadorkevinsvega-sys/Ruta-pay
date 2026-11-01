mod login;
mod registration;

use crate::identity_access_acceptance_context::AcceptanceContext;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
    login::verify(ctx).await;
    registration::verify(ctx).await;
}
