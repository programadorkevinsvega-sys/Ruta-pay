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
use uuid::Uuid;

#[derive(Clone, Default)]
pub(super) struct EventCapture(pub(super) Arc<Mutex<Vec<HashMap<String, String>>>>);

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

pub(super) fn is_stream_not_found(error: async_nats::jetstream::context::GetStreamError) -> bool {
    matches!(
        error.kind(),
        async_nats::jetstream::context::GetStreamErrorKind::JetStream(jetstream_error)
            if jetstream_error.kind() == async_nats::jetstream::ErrorCode::STREAM_NOT_FOUND
    )
}

pub(super) struct EventStreamCleanup {
    pub(super) jetstream: jetstream::Context,
    pub(super) namespace: String,
    pub(super) armed: bool,
}

pub(super) struct AcceptanceDatabaseCleanup {
    pub(super) admin_pool: sqlx::PgPool,
    pub(super) database_name: String,
    pub(super) runtime_role: String,
    armed: bool,
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
    async fn cleanup(&mut self) {
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
