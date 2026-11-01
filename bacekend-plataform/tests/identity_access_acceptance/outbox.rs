mod setup;
mod database_failures;
mod tenant_retry;
mod identity_retry;
mod publish;

use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use uuid::Uuid;
use crate::identity_access_acceptance_support::EventStreamCleanup;

pub(super) struct OutboxInfrastructure {
    pub(super) jetstream: jetstream::Context,
    pub(super) failed_namespace: String,
    pub(super) failed_topology: EventTopology,
    pub(super) event_namespace: String,
    pub(super) event_topology: EventTopology,
}

pub(super) struct OutboxState {
    pub(super) jetstream: jetstream::Context,
    pub(super) event_topology: EventTopology,
    pub(super) stream: jetstream::stream::Stream,
    pub(super) event_stream_cleanup: EventStreamCleanup,
    pub(super) isolated_stream_cleanup: EventStreamCleanup,
    pub(super) registration_subject: String,
    pub(super) registration_event_id: Uuid,
}

pub(super) async fn run(
    migration_pool: &sqlx::PgPool,
    runtime_pool: &sqlx::PgPool,
    runtime_role: &str,
    created_tenant_id: Uuid,
) -> OutboxState {
    let infrastructure = setup::prepare().await;
    database_failures::verify(migration_pool, runtime_pool, runtime_role, &infrastructure).await;
    tenant_retry::verify(migration_pool, runtime_pool, created_tenant_id, &infrastructure).await;
    identity_retry::verify(migration_pool, runtime_pool, &infrastructure).await;
    publish::publish(infrastructure, migration_pool, runtime_pool, created_tenant_id).await
}
