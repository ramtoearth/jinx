//! `StorageError` — unified error type for all storage operations.

use thiserror::Error;

/// All error conditions that the Almacén can report. Every variant maps to a
/// canonical machine-readable `code` string that travels over the IPC Canal as
/// `{ "code": "...", "message": "..." }`.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum StorageError {
    #[error("NOT_FOUND: {0}")]
    NotFound(String),

    #[error("VALIDATION_FAILED: {0}")]
    ValidationFailed(String),

    #[error("GROUP_NAME_NOT_UNIQUE: {0}")]
    GroupNameNotUnique(String),

    #[error("FOREIGN_KEY_VIOLATION: {0}")]
    ForeignKeyViolation(String),

    #[error("IO_NOT_WRITABLE: {0}")]
    IoNotWritable(String),

    #[error("IO_READ_FAILED: {0}")]
    IoReadFailed(String),

    #[error("SCHEMA_MIGRATION_FAILED: {0}")]
    SchemaMigrationFailed(String),

    #[error("VIEWPORT_TOO_SMALL")]
    ViewportTooSmall,

    #[error("INTERNAL_ERROR: {0}")]
    Internal(String),
}

impl StorageError {
    /// The machine-readable code sent over the IPC Canal.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "NOT_FOUND",
            Self::ValidationFailed(_) => "VALIDATION_FAILED",
            Self::GroupNameNotUnique(_) => "GROUP_NAME_NOT_UNIQUE",
            Self::ForeignKeyViolation(_) => "FOREIGN_KEY_VIOLATION",
            Self::IoNotWritable(_) => "IO_NOT_WRITABLE",
            Self::IoReadFailed(_) => "IO_READ_FAILED",
            Self::SchemaMigrationFailed(_) => "SCHEMA_MIGRATION_FAILED",
            Self::ViewportTooSmall => "VIEWPORT_TOO_SMALL",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }

    /// Human-readable description (the part after the colon in `Display`).
    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::SqliteFailure(ref err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                let msg = e.to_string();
                if msg.contains("UNIQUE") && msg.contains("name") {
                    StorageError::GroupNameNotUnique(msg)
                } else if msg.contains("FOREIGN KEY") {
                    StorageError::ForeignKeyViolation(msg)
                } else {
                    StorageError::Internal(msg)
                }
            }
            _ => StorageError::Internal(e.to_string()),
        }
    }
}
