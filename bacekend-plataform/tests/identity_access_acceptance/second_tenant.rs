mod setup;
mod concurrency;
mod isolation;

use crate::identity_access_acceptance_context::AcceptanceContext;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
    setup::verify(ctx).await;
    concurrency::verify(ctx).await;
    isolation::verify(ctx).await;
}
