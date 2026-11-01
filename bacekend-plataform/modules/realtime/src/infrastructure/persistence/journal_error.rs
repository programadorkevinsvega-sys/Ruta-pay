use thiserror::Error;

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("integration event envelope is invalid")]
    InvalidEnvelope,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("realtime signal publication failed")]
    Redis,
    #[error("journal event has an invalid persisted format")]
    InvalidPersistedEvent,
    #[error("realtime cursor is outside the retained scope")]
    CursorOutOfRange,
    #[error("realtime journal contains a cursor gap")]
    CursorGap,
}
