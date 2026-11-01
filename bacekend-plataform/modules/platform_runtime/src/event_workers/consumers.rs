use async_nats::jetstream::{self, consumer::AckPolicy};
use std::time::Duration;

pub(super) async fn ensure_realtime_consumer(
    stream: &jetstream::stream::Stream,
    name: &str,
    filter_subject: &str,
) -> Result<jetstream::consumer::PullConsumer, ()> {
    stream
        .get_or_create_consumer(
            name,
            jetstream::consumer::pull::Config {
                durable_name: Some(name.to_owned()),
                filter_subject: filter_subject.to_owned(),
                deliver_policy: jetstream::consumer::DeliverPolicy::New,
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                max_deliver: 20,
                max_ack_pending: 1024,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| ())
}

pub(super) async fn ensure_membership_consumer(
    stream: &jetstream::stream::Stream,
    name: &str,
    filter_subject: &str,
) -> Result<jetstream::consumer::PullConsumer, ()> {
    stream
        .get_or_create_consumer(
            name,
            jetstream::consumer::pull::Config {
                durable_name: Some(name.to_owned()),
                filter_subject: filter_subject.to_owned(),
                deliver_policy: jetstream::consumer::DeliverPolicy::New,
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                max_deliver: 20,
                max_ack_pending: 512,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| ())
}
