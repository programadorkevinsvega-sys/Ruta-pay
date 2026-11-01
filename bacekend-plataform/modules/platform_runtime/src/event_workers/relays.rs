use async_nats::jetstream;
use sqlx::PgPool;
use std::time::Duration;
use tokio::{sync::watch, time};

use platform_catalog_pricing::infrastructure::outbox_relay as catalog_outbox;
use platform_entitlements_quotas::infrastructure::outbox_relay as entitlements_outbox;
use platform_identity_access::infrastructure::outbox_relay as identity_outbox;
use platform_tenant_registry::infrastructure::outbox_relay as tenant_outbox;

pub(super) async fn run_relays(
    pool: PgPool,
    jetstream: jetstream::Context,
    subject_prefix: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval = time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(error) = identity_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "identity_access", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
                if let Err(error) = tenant_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "tenant_registry", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
                if let Err(error) = catalog_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "catalog_pricing", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
                if let Err(error) = entitlements_outbox::relay_batch(&pool, &jetstream, &subject_prefix).await {
                    tracing::error!(event.name = "outbox.relay.failed", component = "entitlements_quotas", operation = "claim_or_mark_outbox", dependency = "postgresql", error_code = error.as_database_error().and_then(|cause| cause.code()).as_deref().unwrap_or("database_error"), error_category = "persistence", correlation_id = "unavailable", retryable = true);
                }
            }
            result = shutdown.changed() => if result.is_err() || *shutdown.borrow() { return; }
        }
    }
}
