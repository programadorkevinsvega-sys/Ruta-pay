use super::fixture::TestFixture;
use async_nats::jetstream::{
    self,
    consumer::{AckPolicy, DeliverPolicy},
};
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use serde_json::Value;
use std::time::Duration;
use tokio_stream::StreamExt;
use uuid::Uuid;

pub async fn verify(fixture: &TestFixture) {
    let url = std::env::var("PLATFORM_NATS_URL").expect("CI provides authenticated JetStream");
    let user = std::env::var("NATS_USER").expect("CI provides NATS user");
    let password = std::env::var("NATS_PASSWORD").expect("CI provides NATS password");
    let client = async_nats::ConnectOptions::new()
        .user_and_password(user, password)
        .connect(url)
        .await
        .expect("authenticated NATS connection succeeds");
    let jetstream = jetstream::new(client);
    let namespace = format!("e2e_phase3_{}", Uuid::new_v4().simple());
    let topology =
        EventTopology::from_namespace(Some(&namespace)).expect("isolated namespace is valid");
    let stream = topology
        .ensure_stream(&jetstream)
        .await
        .expect("isolated JetStream stream is created");

    sqlx::query("UPDATE catalog_pricing.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL")
        .execute(&fixture.database.migration_pool).await.expect("unrelated catalog events are deferred");
    sqlx::query("UPDATE entitlements_quotas.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL")
        .execute(&fixture.database.migration_pool).await.expect("unrelated entitlement events are deferred");
    let catalog_event = Uuid::new_v4();
    let usage_event = Uuid::new_v4();
    sqlx::query("INSERT INTO catalog_pricing.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,1,'catalog.price_version.published',1,'platform',$3,$4)")
        .bind(catalog_event).bind(Uuid::new_v4())
        .bind(serde_json::json!({"price_version_id": fixture.price_one.id}))
        .bind("phase3_jetstream_catalog").execute(&fixture.database.migration_pool).await
        .expect("catalog outbox event is persisted");
    sqlx::query("INSERT INTO entitlements_quotas.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,1,'quota.consumed',1,'tenant',$3,$4)")
        .bind(usage_event).bind(Uuid::new_v4())
        .bind(serde_json::json!({"tenant_id": fixture.tenant_one, "usage_id": Uuid::new_v4()}))
        .bind("phase3_jetstream_entitlement").execute(&fixture.database.migration_pool).await
        .expect("entitlement outbox event is persisted");

    let catalog_count = platform_catalog_pricing::infrastructure::outbox_relay::relay_batch(
        &fixture.database.runtime_pool,
        &jetstream,
        &topology.subject_prefix,
    )
    .await
    .expect("catalog outbox relay succeeds");
    let entitlement_count =
        platform_entitlements_quotas::infrastructure::outbox_relay::relay_batch(
            &fixture.database.runtime_pool,
            &jetstream,
            &topology.subject_prefix,
        )
        .await
        .expect("entitlement outbox relay succeeds");
    assert_eq!(
        catalog_count, 1,
        "catalog event receives JetStream publish acknowledgement"
    );
    assert_eq!(
        entitlement_count, 1,
        "entitlement event receives JetStream publish acknowledgement"
    );

    let consumer = stream
        .get_or_create_consumer(
            "PHASE3_ACCEPTANCE",
            jetstream::consumer::pull::Config {
                durable_name: Some("PHASE3_ACCEPTANCE".to_owned()),
                filter_subject: format!("{}.>", topology.subject_prefix),
                deliver_policy: DeliverPolicy::All,
                ack_policy: AckPolicy::Explicit,
                ..Default::default()
            },
        )
        .await
        .expect("isolated pull consumer is created");
    let mut messages = consumer
        .messages()
        .await
        .expect("consumer message stream opens");
    let mut received = Vec::new();
    for _ in 0..2 {
        let message = tokio::time::timeout(Duration::from_secs(5), messages.next())
            .await
            .expect("JetStream delivers both phase three events before timeout")
            .expect("consumer remains open")
            .expect("JetStream message is valid");
        let envelope: Value =
            serde_json::from_slice(&message.payload).expect("event envelope is JSON");
        received.push((
            envelope["event_id"].as_str().unwrap_or_default().to_owned(),
            envelope["event_type"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            envelope["correlation_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ));
        message.ack().await.expect("received event is acknowledged");
    }
    assert!(
        received
            .iter()
            .any(|event| event.0 == catalog_event.to_string()
                && event.1 == "catalog.price_version.published"
                && event.2 == "phase3_jetstream_catalog")
    );
    assert!(
        received
            .iter()
            .any(|event| event.0 == usage_event.to_string()
                && event.1 == "quota.consumed"
                && event.2 == "phase3_jetstream_entitlement")
    );
    delete_e2e_stream(&jetstream, &namespace)
        .await
        .expect("only this isolated phase three stream is deleted");
}
