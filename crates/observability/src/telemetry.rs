use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource, logs::SdkLoggerProvider, propagation::TraceContextPropagator,
    trace::SdkTracerProvider,
};
use sentry::{ClientInitGuard, ClientOptions};
use std::borrow::Cow;
use thiserror::Error;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub struct TelemetryConfig<'a> {
    pub service_name: &'a str,
    pub environment: &'a str,
    pub release: &'a str,
    pub otlp_endpoint: Option<&'a str>,
    pub sentry_dsn: Option<&'a str>,
}

pub struct TelemetryRuntime {
    pub sentry_guard: ClientInitGuard,
    pub tracer_provider: SdkTracerProvider,
    pub logger_provider: SdkLoggerProvider,
}

#[derive(Debug, Error)]
pub enum TelemetryInitError {
    #[error("telemetry exporter initialization failed")]
    Exporter,
    #[error("telemetry subscriber initialization failed")]
    Subscriber,
}

/// Installs JSON logs, W3C trace propagation, OTLP batch export, and Sentry capture.
/// Remote exporters run outside request handlers; stdout JSON remains active without them.
pub fn initialize(config: &TelemetryConfig<'_>) -> Result<TelemetryRuntime, TelemetryInitError> {
    let mut sentry_options = ClientOptions::default();
    sentry_options.dsn = config.sentry_dsn.and_then(|value| value.parse().ok());
    sentry_options.environment = Some(Cow::Owned(config.environment.to_owned()));
    sentry_options.release = Some(Cow::Owned(config.release.to_owned()));
    sentry_options.send_default_pii = false;
    sentry_options = sentry_options.before_send(sanitize_sentry_event);
    let sentry_guard = sentry::init(sentry_options);
    let resource = Resource::builder()
        .with_service_name(config.service_name.to_owned())
        .build();
    let tracer_provider = tracer_provider(config, resource.clone())?;
    let logger_provider = logger_provider(config, resource)?;
    global::set_tracer_provider(tracer_provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    let sentry_layer =
        sentry::integrations::tracing::layer().event_filter(|metadata| match *metadata.level() {
            tracing::Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
            _ => sentry::integrations::tracing::EventFilter::Breadcrumb,
        });
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().json())
        .with(
            tracing_opentelemetry::layer()
                .with_tracer(tracer_provider.tracer(config.service_name.to_owned())),
        )
        .with(OpenTelemetryTracingBridge::new(&logger_provider))
        .with(sentry_layer)
        .try_init()
        .map_err(|_| TelemetryInitError::Subscriber)?;
    Ok(TelemetryRuntime {
        sentry_guard,
        tracer_provider,
        logger_provider,
    })
}

fn sanitize_sentry_event(
    mut event: sentry::protocol::Event<'static>,
) -> Option<sentry::protocol::Event<'static>> {
    event.message = None;
    event.logentry = None;
    event.transaction = None;
    event.culprit = None;
    event.user = None;
    event.request = None;
    event.contexts.retain(|name, _| name == "trace");
    event.breadcrumbs = Default::default();
    event.tags.clear();
    event.extra.clear();
    for exception in event.exception.iter_mut() {
        exception.value = None;
    }
    Some(event)
}

fn tracer_provider(
    config: &TelemetryConfig<'_>,
    resource: Resource,
) -> Result<SdkTracerProvider, TelemetryInitError> {
    let builder = SdkTracerProvider::builder().with_resource(resource);
    match config.otlp_endpoint {
        Some(endpoint) => {
            let exporter = SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.to_owned())
                .build()
                .map_err(|_| TelemetryInitError::Exporter)?;
            Ok(builder.with_batch_exporter(exporter).build())
        }
        None => Ok(builder.build()),
    }
}

fn logger_provider(
    config: &TelemetryConfig<'_>,
    resource: Resource,
) -> Result<SdkLoggerProvider, TelemetryInitError> {
    let builder = SdkLoggerProvider::builder().with_resource(resource);
    match config.otlp_endpoint {
        Some(endpoint) => {
            let exporter = LogExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.to_owned())
                .build()
                .map_err(|_| TelemetryInitError::Exporter)?;
            Ok(builder.with_batch_exporter(exporter).build())
        }
        None => Ok(builder.build()),
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_sentry_event;
    use sentry::protocol::{Event, Exception};

    #[test]
    fn sentry_events_drop_messages_and_context_that_may_contain_private_values() {
        let mut event = Event::new();
        event.message = Some("token=private-value".to_owned());
        event.extra.insert(
            "authorization".to_owned(),
            serde_json::Value::String("Bearer private-value".to_owned()),
        );
        event.exception.values.push(Exception {
            ty: "ProviderError".to_owned(),
            value: Some("secret=private-value".to_owned()),
            ..Exception::default()
        });

        let sanitized = sanitize_sentry_event(event).expect("event remains reportable");
        let serialized = serde_json::to_string(&sanitized).expect("event serializes");

        assert!(!serialized.contains("private-value"));
        assert!(serialized.contains("ProviderError"));
        assert!(sanitized.message.is_none());
        assert!(sanitized.extra.is_empty());
        assert_eq!(sanitized.exception[0].value, None);
    }
}
