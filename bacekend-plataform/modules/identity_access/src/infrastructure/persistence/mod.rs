pub const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
    table_name: std::borrow::Cow::Borrowed("identity_access._sqlx_migrations"),
    create_schemas: std::borrow::Cow::Borrowed(&[std::borrow::Cow::Borrowed("identity_access")]),
    ..sqlx::migrate!("infrastructure/persistence/migrations")
};
