mod origin_validation;
mod persistence;
mod authentication;
mod sessions;

use crate::identity_access_acceptance_context::AcceptanceContext;

pub(super) async fn verify(ctx: &mut AcceptanceContext) {
    origin_validation::verify(ctx).await;
    persistence::verify(ctx).await;
    authentication::verify(ctx).await;
    sessions::verify(ctx).await;
}
