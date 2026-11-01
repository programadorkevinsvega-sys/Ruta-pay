mod registration;
mod status;

use crate::identity_access_acceptance_context::AcceptanceContext;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
    registration::verify(ctx).await;
    status::verify(ctx).await;
}
