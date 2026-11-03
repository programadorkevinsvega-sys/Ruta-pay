use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tracing::{
    Event,
    field::{Field, Visit},
};
use tracing_subscriber::{
    layer::{Context, Layer},
    prelude::*,
    registry::LookupSpan,
};

#[derive(Clone, Default)]
struct EventCapture(Arc<Mutex<Vec<HashMap<String, String>>>>);

impl<S> Layer<S> for EventCapture
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut fields = HashMap::new();
        event.record(&mut FieldCapture(&mut fields));
        if let Some(span) = context.lookup_current() {
            fields.insert("current_span".to_owned(), span.name().to_owned());
        }
        self.0.lock().expect("event capture lock").push(fields);
    }
}

struct FieldCapture<'a>(&'a mut HashMap<String, String>);

impl Visit for FieldCapture<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

pub type CapturedEvents = Arc<Mutex<Vec<HashMap<String, String>>>>;

pub fn initialize_capture() -> (SdkTracerProvider, CapturedEvents) {
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    let captured = EventCapture::default();
    let capture_store = captured.0.clone();
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("acceptance")))
        .with(captured)
        .try_init()
        .expect("acceptance subscriber initializes once");
    (provider, capture_store)
}
