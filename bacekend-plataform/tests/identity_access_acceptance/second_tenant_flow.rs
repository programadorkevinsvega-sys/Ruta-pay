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

pub async fn verify(
    client: &Client,
    migration_pool: &PgPool,
    runtime_pool: &PgPool,
    jetstream: &jetstream::Context,
    event_topology: &EventTopology,
    stream: &jetstream::stream::Stream,
    registration_subject: &str,
    http_addr: SocketAddr,
    origin: &str,
    tenant_slug: &str,
    created_tenant_id: Uuid,
) -> SecondTenantFixture {
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
        &second_tenant_credentials,
    )
    .await;

    SecondTenantFixture {
        credentials: second_tenant_credentials,
        tenant_id: second_tenant_id,
    }
}
