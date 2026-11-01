mod phase3_catalog_entitlements_acceptance {
    mod concurrency;
    mod database;
    mod fixture;
    mod isolation;
    mod quota;
    mod versioning;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn phase_three_real_postgres_limits_replay_concurrency_versioning_and_rls() {
        let fixture = fixture::TestFixture::new().await;
        quota::verify(&fixture).await;
        concurrency::verify(&fixture).await;
        versioning::verify(&fixture).await;
        isolation::verify(&fixture).await;
        fixture.finish().await;
    }
}
