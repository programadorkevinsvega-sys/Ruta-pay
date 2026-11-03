use super::*;
use async_nats::jetstream;
use platform_runtime::event_topology::EventTopology;
use reqwest::Client;
use sqlx::PgPool;
use std::net::SocketAddr;
use uuid::Uuid;

use super::second_tenant_registration::SecondTenantCredentials;

pub struct SecondTenantFixture {
    pub credentials: SecondTenantCredentials,
    pub tenant_id: Uuid,
}

pub struct SecondTenantFlowContext<'a> {
    pub client: &'a Client,
    pub migration_pool: &'a PgPool,
    pub runtime_pool: &'a PgPool,
    pub jetstream: &'a jetstream::Context,
    pub event_topology: &'a EventTopology,
    pub stream: &'a jetstream::stream::Stream,
    pub registration_subject: &'a str,
    pub http_addr: SocketAddr,
    pub origin: &'a str,
    pub tenant_slug: &'a str,
    pub created_tenant_id: Uuid,
}

pub async fn verify(context: SecondTenantFlowContext<'_>) -> SecondTenantFixture {
    let SecondTenantFlowContext {
        client, migration_pool, runtime_pool, jetstream, event_topology, stream,
        registration_subject, http_addr, origin, tenant_slug, created_tenant_id,
    } = context;
    let second_tenant_credentials = second_tenant_registration::register(
        client,
        migration_pool,
        http_addr,
        origin,
        tenant_slug,
        created_tenant_id,
    )
    .await;
    let second_tenant_id = second_tenant_isolation::create_and_verify(
        second_tenant_isolation::SecondTenantIsolationContext {
            client,
            migration_pool,
            runtime_pool,
            jetstream,
            event_topology,
            stream,
            registration_subject,
            http_addr,
            origin,
            created_tenant_id,
            credentials: &second_tenant_credentials,
        },
    )
    .await;

    SecondTenantFixture {
        credentials: second_tenant_credentials,
        tenant_id: second_tenant_id,
    }
}
