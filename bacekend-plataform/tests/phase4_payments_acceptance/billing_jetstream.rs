use async_nats::jetstream::{self, consumer::{AckPolicy, DeliverPolicy}};
use chrono::{Duration, Utc};
use platform_billing::{
    application::{BillingService, CreateInvoice, InvoiceLine},
    infrastructure::outbox_relay,
};
use platform_runtime::event_topology::{EventTopology, delete_e2e_stream};
use serde_json::Value;
use sqlx::Row;
use std::time::Duration as StdDuration;
use tokio_stream::StreamExt;
use uuid::Uuid;

use super::database::TestDatabase;

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn billing_outbox_publishes_versioned_fiscal_event_and_recovers_ack_window(){
    let database=TestDatabase::new().await;
    let tenant=Uuid::new_v4();
    let now=Utc::now();
    let service=BillingService::new(database.runtime_pool.clone());
    let invoice=service.create_draft(CreateInvoice{tenant_id:tenant,invoice_number:"PV-JS-0001",
        currency:"COP",period_start:now-Duration::days(1),period_end:now,due_at:None,
        correlation_id:"billing-jetstream-acceptance"}).await.expect("invoice draft created");
    service.add_line(tenant,invoice,InvoiceLine{description:"approved SaaS amount",meter_key:None,
        source_usage_event_id:None,quantity:1,unit:"month",unit_price_minor:25000})
        .await.expect("explicit approved price is recorded");
    service.issue_invoice(tenant,invoice,"billing-jetstream-acceptance")
        .await.expect("invoice and fiscal event commit atomically");
    let stored=sqlx::query("SELECT event_id,occurred_at FROM platform_billing.outbox_events WHERE aggregate_id=$1")
        .bind(invoice).fetch_one(&database.migration_pool).await.expect("fiscal outbox row exists");
    let event_id:Uuid=stored.try_get("event_id").expect("event id");
    let occurred_at:chrono::DateTime<Utc>=stored.try_get("occurred_at").expect("event timestamp");

    let url=std::env::var("PLATFORM_NATS_URL").expect("CI provides NATS");
    let username=std::env::var("NATS_USER").expect("CI provides NATS user");
    let password=std::env::var("NATS_PASSWORD").expect("CI provides NATS password");
    let client=async_nats::ConnectOptions::new().user_and_password(username,password)
        .connect(url).await.expect("authenticated NATS connection");
    let jetstream=jetstream::new(client);
    let namespace=format!("e2e_billing_{}",Uuid::new_v4().simple());
    let topology=EventTopology::from_namespace(Some(&namespace)).expect("isolated namespace");
    let mut stream=topology.ensure_stream(&jetstream).await.expect("billing stream created");
    let count=outbox_relay::relay_batch(&database.runtime_pool,&jetstream,&topology.subject_prefix)
        .await.expect("billing relay publishes invoice event");
    assert_eq!(count,1);
    let consumer_name=format!("BILLING_ACCEPTANCE_{}",Uuid::new_v4().simple());
    let consumer=stream.get_or_create_consumer(&consumer_name,jetstream::consumer::pull::Config{
        durable_name:Some(consumer_name.clone()),
        filter_subject:format!("{}.billing.invoice.issued",topology.subject_prefix),
        deliver_policy:DeliverPolicy::All,ack_policy:AckPolicy::Explicit,..Default::default()
    }).await.expect("durable billing consumer created");
    let mut messages=consumer.messages().await.expect("consumer messages open");
    let message=tokio::time::timeout(StdDuration::from_secs(5),messages.next()).await
        .expect("event arrives").expect("consumer stays open").expect("valid event");
    let envelope:Value=serde_json::from_slice(&message.payload).expect("event envelope JSON");
    assert_eq!(envelope["event_id"],event_id.to_string());
    assert_eq!(envelope["event_type"],"billing.invoice.issued");
    assert_eq!(envelope["audience"],"fiscal_integration");
    assert_eq!(envelope["occurred_at"],occurred_at.to_rfc3339());
    assert_eq!(envelope["payload"]["internal_invoice_id"],invoice.to_string());
    assert_eq!(envelope["payload"]["tenant_id"],tenant.to_string());
    assert_eq!(envelope["payload"]["snapshot_schema_version"],1);
    assert_eq!(envelope["payload"]["snapshot_media_type"],"application/json");
    assert!(envelope["payload"]["immutable_invoice_snapshot"].as_str().is_some());
    assert!(envelope["payload"]["snapshot_sha256"].as_str().is_some());
    message.ack().await.expect("consumer acknowledges durable event");

    sqlx::query("UPDATE platform_billing.outbox_events SET published_at=NULL,available_at=now(),lease_token=NULL,lease_until=NULL WHERE event_id=$1")
        .bind(event_id).execute(&database.migration_pool).await.expect("simulate ACK-before-DB-ack crash window");
    assert_eq!(outbox_relay::relay_batch(&database.runtime_pool,&jetstream,&topology.subject_prefix)
        .await.expect("same event can be safely redelivered"),1);
    let state=sqlx::query("SELECT attempts,published_at FROM platform_billing.outbox_events WHERE event_id=$1")
        .bind(event_id).fetch_one(&database.migration_pool).await.expect("delivery state");
    assert_eq!(state.try_get::<i32,_>("attempts").expect("attempts"),2);
    assert!(state.try_get::<Option<chrono::DateTime<Utc>>,_>("published_at").expect("published at").is_some());
    assert_eq!(stream.info().await.expect("stream info").state.messages,1,"Nats-Msg-Id deduplicates retry");

    drop(message);drop(messages);drop(consumer);
    delete_e2e_stream(&jetstream,&namespace).await.expect("isolated billing stream removed");
    database.finish().await;
}
