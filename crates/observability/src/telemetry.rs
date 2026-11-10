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
    pub sentry_traces_sample_rate: Option<f32>,
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
    #[error("Sentry DSN is invalid")]
    InvalidSentryDsn,
}

/// Installs JSON logs, W3C trace propagation, OTLP batch export, and Sentry capture.
/// Remote exporters run outside request handlers; stdout JSON remains active without them.
pub fn initialize(config: &TelemetryConfig<'_>) -> Result<TelemetryRuntime, TelemetryInitError> {
    let mut sentry_options = ClientOptions::default();
    sentry_options.dsn = config
        .sentry_dsn
        .map(str::parse)
        .transpose()
        .map_err(|_| TelemetryInitError::InvalidSentryDsn)?;
    sentry_options.environment = Some(Cow::Owned(config.environment.to_owned()));
    sentry_options.release = Some(Cow::Owned(config.release.to_owned()));
    sentry_options.send_default_pii = false;
    if let Some(sample_rate) = config.sentry_traces_sample_rate {
        sentry_options = sentry_options.traces_sample_rate(sample_rate);
    }
    sentry_options = sentry_options.before_send(sanitize_sentry_event);
    let sentry_guard = sentry::init(sentry_options);
    let resource = Resource::builder()
        .with_service_name(config.service_name.to_owned())
        .build();
    let tracer_provider = tracer_provider(config, resource.clone())?;
    let logger_provider = logger_provider(config, resource)?;
    global::set_tracer_provider(tracer_provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    let sentry_layer = sentry::integrations::tracing::layer().event_filter(|metadata| {
        if *metadata.level() == tracing::Level::ERROR {
            sentry::integrations::tracing::EventFilter::Event
        } else {
            sentry::integrations::tracing::EventFilter::Breadcrumb
        }
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
    event.logger = None;
    event.modules.clear();
    event.server_name = None;
    event.dist = None;
    event.user = None;
    event.request = None;
    event.contexts.retain(|name, _| name == "trace");
    event.breadcrumbs = Default::default();
    event.tags.clear();
    event.extra.clear();
    event.threads = Default::default();
    event.stacktrace = None;
    event.template = None;
    event.debug_meta = Default::default();
    event.exception.values.retain_mut(|exception| {
        exception.value = None;
        exception.raw_stacktrace = None;
        exception.mechanism = None;
        exception.thread_id = None;
        if let Some(stacktrace) = exception.stacktrace.as_mut() {
            sanitize_stacktrace(stacktrace);
        }
        if exception.ty.is_empty()
            || exception.ty.len() > 96
            || !exception
                .ty
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_:.-".contains(&byte))
        {
            return false;
        }
        true
    });
    Some(event)
}

fn sanitize_stacktrace(stacktrace: &mut sentry::protocol::Stacktrace) {
    stacktrace.registers.clear();
    for frame in &mut stacktrace.frames {
        frame.function = None;
        frame.symbol = None;
        frame.module = None;
        frame.package = None;
        frame.filename = None;
        frame.abs_path = None;
        frame.pre_context.clear();
        frame.context_line = None;
        frame.post_context.clear();
        frame.vars.clear();
    }
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
    use sentry::protocol::{Event, Exception, Frame, Stacktrace};

    #[test]
    fn sentry_events_keep_safe_error_class_but_drop_private_values_and_context() {
        let mut event = Event::new();
        event.message = Some("token=private-value".to_owned());
        event.logger = Some("logger-private-value".to_owned());
        event.server_name = Some("host-private-value".into());
        event.modules.insert(
            "module-private-value".to_owned(),
            "version-private-value".to_owned(),
        );
        event.extra.insert(
            "authorization".to_owned(),
            serde_json::Value::String("Bearer private-value".to_owned()),
        );
        event.exception.values.push(Exception {
            ty: "ProviderError".to_owned(),
            value: Some("secret=private-value".to_owned()),
            stacktrace: Some(Stacktrace {
                frames: vec![Frame {
                    function: Some("function-private-value".to_owned()),
                    symbol: Some("symbol-private-value".to_owned()),
                    module: Some("module-private-value".to_owned()),
                    package: Some("package-private-value".to_owned()),
                    filename: Some("filename-private-value".to_owned()),
                    vars: serde_json::from_value(serde_json::json!({
                        "credential": "private-value"
                    }))
                    .expect("frame variables"),
                    context_line: Some("token=private-value".to_owned()),
                    ..Frame::default()
                }],
                ..Stacktrace::default()
            }),
            ..Exception::default()
        });

        let sanitized = sanitize_sentry_event(event).expect("event remains reportable");
        let serialized = serde_json::to_string(&sanitized).expect("event serializes");

        assert!(!serialized.contains("private-value"));
        assert!(serialized.contains("ProviderError"));
        assert!(sanitized.message.is_none());
        assert!(sanitized.logger.is_none());
        assert!(sanitized.server_name.is_none());
        assert!(sanitized.modules.is_empty());
        assert!(sanitized.extra.is_empty());
        assert_eq!(sanitized.exception[0].value, None);
        let stacktrace = sanitized.exception[0]
            .stacktrace
            .as_ref()
            .expect("stacktrace remains");
        assert!(stacktrace.frames[0].vars.is_empty());
        assert!(stacktrace.frames[0].context_line.is_none());
        assert!(stacktrace.frames[0].function.is_none());
        assert!(stacktrace.frames[0].symbol.is_none());
        assert!(stacktrace.frames[0].module.is_none());
        assert!(stacktrace.frames[0].package.is_none());
        assert!(stacktrace.frames[0].filename.is_none());
    }

    #[test]
    fn sentry_events_drop_exception_types_that_could_contain_private_values() {
        let mut event = Event::new();
        event.exception.values.push(Exception {
            ty: "ProviderError account=private-value".to_owned(),
            value: Some("private-value".to_owned()),
            ..Exception::default()
        });

        let sanitized = sanitize_sentry_event(event).expect("event remains reportable");
        let serialized = serde_json::to_string(&sanitized).expect("event serializes");
        assert!(!serialized.contains("private-value"));
        assert!(sanitized.exception.is_empty());
    }
}
