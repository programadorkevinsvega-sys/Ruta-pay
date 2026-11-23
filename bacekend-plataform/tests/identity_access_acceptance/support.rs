use async_nats::jetstream;
use platform_runtime::event_topology::delete_e2e_stream;
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
    registry::LookupSpan,
};

#[derive(Clone, Default)]
pub(super) struct EventCapture(pub(super) Arc<Mutex<Vec<HashMap<String, String>>>>);

pub(super) fn assert_http_failure_diagnostics(
    captured_events: &Arc<Mutex<Vec<HashMap<String, String>>>>,
    correlation_id: &str,
    error_code: &str,
    error_category: &str,
) {
    let events = captured_events.lock().expect("event capture lock").clone();
    let http_event = events
        .iter()
        .rev()
        .find(|event| {
            event.get("event.name").map(String::as_str) == Some("http.request.completed")
                && event.get("correlation_id").map(String::as_str) == Some(correlation_id)
        })
        .expect("HTTP completion diagnostic is emitted");
    let trace_id = http_event
        .get("trace_id")
        .expect("HTTP completion contains trace id");
    assert!(!trace_id.is_empty());
    assert_eq!(
        http_event.get("error_code").map(String::as_str),
        Some(error_code)
    );
    assert_eq!(
        http_event.get("error_category").map(String::as_str),
        Some(error_category)
    );

    for event_name in [
        "grpc.dependency.call.failed",
        "grpc.module.request.failed",
        "bff.dependency.call.failed",
        "bff.request.failed",
    ] {
        let event = events
            .iter()
            .rev()
            .find(|event| {
                event.get("event.name").map(String::as_str) == Some(event_name)
                    && event.get("correlation_id").map(String::as_str) == Some(correlation_id)
                    && event.get("trace_id") == Some(trace_id)
            })
            .unwrap_or_else(|| panic!("{event_name} diagnostic matches the HTTP request context"));
        assert_eq!(
            event.get("error_code").map(String::as_str),
            Some(error_code),
            "{event_name} preserves the stable error code"
        );
        assert_eq!(
            event.get("error_category").map(String::as_str),
            Some(error_category),
            "{event_name} preserves the stable error category"
        );
    }
}

pub(super) fn safe_diagnostics_for_correlation(
    captured_events: &Arc<Mutex<Vec<HashMap<String, String>>>>,
    correlation_id: &str,
) -> Vec<HashMap<String, String>> {
    let fields = [
        "event.name",
        "component",
        "operation",
        "dependency",
        "grpc_code",
        "error_code",
        "error_category",
        "error_class",
        "sqlstate",
        "constraint",
        "table",
        "column",
        "dependency_duration_ms",
        "duration_total_ms",
        "correlation_id",
        "trace_id",
        "span_id",
    ];
    captured_events
        .lock()
        .expect("event capture lock")
        .iter()
        .filter(|event| event.get("correlation_id").map(String::as_str) == Some(correlation_id))
        .map(|event| {
            fields
                .iter()
                .filter_map(|field| {
                    event
                        .get(*field)
                        .map(|value| ((*field).to_owned(), value.clone()))
                })
                .collect()
        })
        .collect()
}

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

pub fn is_stream_not_found(error: async_nats::jetstream::context::GetStreamError) -> bool {
    matches!(
        error.kind(),
        async_nats::jetstream::context::GetStreamErrorKind::JetStream(jetstream_error)
            if jetstream_error.kind() == async_nats::jetstream::ErrorCode::STREAM_NOT_FOUND
    )
}

pub struct EventStreamCleanup {
    pub(super) jetstream: jetstream::Context,
    pub(super) namespace: String,
    pub(super) armed: bool,
}

pub struct AcceptanceDatabaseCleanup {
    pub(super) admin_pool: sqlx::PgPool,
    pub(super) database_name: String,
    pub(super) runtime_role: String,
    pub(super) armed: bool,
}

impl AcceptanceDatabaseCleanup {
    pub(super) async fn cleanup(&mut self) {
        Self::drop_owned_resources(&self.admin_pool, &self.database_name, &self.runtime_role)
            .await
            .expect("acceptance database and role are removed");
        self.armed = false;
    }

    async fn drop_owned_resources(
        admin_pool: &sqlx::PgPool,
        database_name: &str,
        runtime_role: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
        )
        .bind(database_name)
        .execute(admin_pool)
        .await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{database_name}\""
        )))
        .execute(admin_pool)
        .await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP ROLE IF EXISTS \"{runtime_role}\""
        )))
        .execute(admin_pool)
        .await?;
        Ok(())
    }
}

impl Drop for AcceptanceDatabaseCleanup {
    fn drop(&mut self) {
        if self.armed {
            let admin_pool = self.admin_pool.clone();
            let database_name = self.database_name.clone();
            let runtime_role = self.runtime_role.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                runtime.block_on(async move {
                    Self::drop_owned_resources(&admin_pool, &database_name, &runtime_role)
                        .await
                        .expect("panic cleanup removes only this acceptance database and role");
                })
            });
        }
    }
}

impl EventStreamCleanup {
    pub(super) async fn cleanup(&mut self) {
        delete_e2e_stream(&self.jetstream, &self.namespace)
            .await
            .expect("only the integration test stream is deleted");
        self.armed = false;
    }
}

impl Drop for EventStreamCleanup {
    fn drop(&mut self) {
        if self.armed {
            let jetstream = self.jetstream.clone();
            let namespace = self.namespace.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                runtime.block_on(async move {
                    delete_e2e_stream(&jetstream, &namespace)
                        .await
                        .expect("panic cleanup deletes only this integration stream");
                })
            });
        }
    }
}

impl Visit for FieldCapture<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

pub(super) fn cookie_value(cookies: &[String], name: &str) -> String {
    cookies
        .iter()
        .find_map(|cookie| {
            cookie
                .strip_prefix(&format!("{name}="))
                .and_then(|cookie| cookie.split(';').next())
                .map(str::to_owned)
        })
        .expect("required session cookie is present")
}
