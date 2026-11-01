#[path = "phase3_catalog_entitlements_acceptance/atomicity.rs"]
mod atomicity;
#[path = "phase3_catalog_entitlements_acceptance/concurrency.rs"]
mod concurrency;
#[path = "phase3_catalog_entitlements_acceptance/database.rs"]
mod database;
#[path = "phase3_catalog_entitlements_acceptance/fixture.rs"]
mod fixture;
#[path = "phase3_catalog_entitlements_acceptance/immutability.rs"]
mod immutability;
#[path = "phase3_catalog_entitlements_acceptance/isolation.rs"]
mod isolation;
#[path = "phase3_catalog_entitlements_acceptance/jetstream.rs"]
mod jetstream;
#[path = "phase3_catalog_entitlements_acceptance/quota.rs"]
mod quota;
#[path = "phase3_catalog_entitlements_acceptance/versioning.rs"]
mod versioning;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phase_three_real_postgres_limits_replay_concurrency_versioning_rls_and_integrity() {
    let fixture = fixture::TestFixture::new().await;
    quota::verify(&fixture).await;
    concurrency::verify(&fixture).await;
    versioning::verify(&fixture).await;
    immutability::verify(&fixture).await;
    atomicity::verify(&fixture).await;
    jetstream::verify(&fixture).await;
    isolation::verify(&fixture).await;
    fixture.finish().await;
}
