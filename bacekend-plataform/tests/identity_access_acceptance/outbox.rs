mod setup;
mod database_failures;
mod tenant_retry;
mod identity_retry;
mod publish;

use platform_runtime::event_topology::EventTopology;
use async_nats::jetstream;

pub(super) struct OutboxInfrastructure {
    pub(super) jetstream: jetstream::Context,
    pub(super) failed_namespace: String,
    pub(super) failed_topology: EventTopology,
    pub(super) event_namespace: String,
    pub(super) event_topology: EventTopology,
}

pub(super) async fn verify(ctx: &mut crate::identity_access_acceptance_context::AcceptanceContext) {
    let infrastructure = setup::prepare().await;
    database_failures::verify(ctx, &infrastructure).await;
    tenant_retry::verify(ctx, &infrastructure).await;
    identity_retry::verify(ctx, &infrastructure).await;
    publish::verify(ctx, infrastructure).await;
}
