use async_nats::jetstream::{self, stream::RetentionPolicy};
use std::time::Duration;

const PRODUCTION_STREAM: &str = "PAYVIA_PLATFORM_EVENTS";
const PRODUCTION_SUBJECT_PREFIX: &str = "payvia.platform.events.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventTopology {
    namespace: Option<String>,
    pub stream_name: String,
    pub subject_prefix: String,
    pub membership_consumer: String,
    pub realtime_consumer: String,
}

impl EventTopology {
    pub fn from_namespace(namespace: Option<&str>) -> Result<Self, &'static str> {
        let namespace = namespace
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let Some(value) = namespace.as_deref()
            && (value.len() > 48
                || !value.bytes().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || character == b'_'
                })
                || !value.as_bytes()[0].is_ascii_lowercase())
        {
            return Err("event_namespace_invalid");
        }
        let (stream_name, subject_prefix, consumer_suffix) = match namespace.as_deref() {
            Some(value) => (
                format!("PAYVIA_PLATFORM_EVENTS_{}", value.to_ascii_uppercase()),
                format!("payvia.platform.events.{value}.v1"),
                format!("_{}", value.to_ascii_uppercase()),
            ),
            None => (
                PRODUCTION_STREAM.to_owned(),
                PRODUCTION_SUBJECT_PREFIX.to_owned(),
                String::new(),
            ),
        };
        Ok(Self {
            namespace,
            stream_name,
            subject_prefix,
            membership_consumer: format!("PLATFORM_TENANT_OWNER_PROVISIONER{consumer_suffix}"),
            realtime_consumer: format!("PLATFORM_REALTIME_JOURNAL{consumer_suffix}"),
        })
    }

    pub fn is_e2e(&self) -> bool {
        self.namespace
            .as_deref()
            .is_some_and(|namespace| namespace.starts_with("e2e_"))
    }

    pub async fn ensure_stream(
        &self,
        jetstream: &jetstream::Context,
    ) -> Result<jetstream::stream::Stream, &'static str> {
        jetstream
            .get_or_create_stream(jetstream::stream::Config {
                name: self.stream_name.clone(),
                subjects: vec![format!("{}.>", self.subject_prefix)],
                retention: RetentionPolicy::Limits,
                storage: jetstream::stream::StorageType::File,
                duplicate_window: Duration::from_secs(120),
                max_age: Duration::from_secs(90 * 24 * 60 * 60),
                max_messages: 10_000_000,
                ..Default::default()
            })
            .await
            .map_err(|_| "jetstream_stream_unavailable")
    }
}

pub async fn delete_e2e_stream(
    jetstream: &jetstream::Context,
    namespace: &str,
) -> Result<bool, &'static str> {
    let topology =
        EventTopology::from_namespace(Some(namespace)).map_err(|_| "event_namespace_invalid")?;
    if !topology.is_e2e() {
        return Err("event_namespace_not_e2e");
    }
    match jetstream.delete_stream(&topology.stream_name).await {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                async_nats::jetstream::context::GetStreamErrorKind::JetStream(jetstream_error)
                    if jetstream_error.kind() == async_nats::jetstream::ErrorCode::STREAM_NOT_FOUND
            ) =>
        {
            Ok(false)
        }
        Err(_) => Err("jetstream_stream_cleanup_failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::EventTopology;

    #[test]
    fn namespace_isolates_stream_subjects_and_consumers() {
        let first = EventTopology::from_namespace(Some("e2e_a1")).expect("valid namespace");
        let second = EventTopology::from_namespace(Some("e2e_b2")).expect("valid namespace");

        assert_ne!(first.stream_name, second.stream_name);
        assert_ne!(first.subject_prefix, second.subject_prefix);
        assert_ne!(first.membership_consumer, second.membership_consumer);
        assert_ne!(first.realtime_consumer, second.realtime_consumer);
        assert!(first.is_e2e());
    }

    #[test]
    fn no_namespace_preserves_production_topology_and_invalid_tokens_fail() {
        let production = EventTopology::from_namespace(None).expect("production topology");

        assert_eq!(production.stream_name, "PAYVIA_PLATFORM_EVENTS");
        assert_eq!(production.subject_prefix, "payvia.platform.events.v1");
        assert!(!production.is_e2e());
        assert!(EventTopology::from_namespace(Some("e2e;delete-all")).is_err());
        assert!(EventTopology::from_namespace(Some("_e2e")).is_err());
    }
}
