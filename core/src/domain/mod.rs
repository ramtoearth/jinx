pub mod calendar;
pub mod finance;
pub mod group;
pub mod note;
pub mod shared;
pub mod task;

pub use calendar::{Event, EventPatch, EventRepository, NewEvent};
pub use finance::{
    Budget, Debt, DebtPatch, FinCategory, FinanceRepository, Goal, GoalHorizon, GoalPatch,
    MonthlySummary, NewBudget, NewCategory, NewDebt, NewGoal, NewRecurringRule, NewTransaction,
    RecurringPeriod, RecurringRule, Transaction, TransactionFilter, TransactionType,
};
pub use group::{Group, GroupInfo, GroupPatch, GroupRepository, GroupsSnapshot, NewGroup};
pub use note::{NewNote, Note, NoteRepository, NotePatch};
pub use shared::HexColor;
pub use task::{
    NewTask, Priority, Task, TaskFilter, TaskPatch, TaskRepository, TaskStatus,
};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("validation failed: {0}")]
    ValidationFailed(String),

    #[error("group name not unique: {0}")]
    GroupNameNotUnique(String),

    #[error("foreign key violation: {0}")]
    ForeignKeyViolation(String),

    #[error("I/O error: {0}")]
    IoError(String),

    #[error("schema migration failed: {0}")]
    SchemaMigrationFailed(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl DomainError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "NOT_FOUND",
            Self::ValidationFailed(_) => "VALIDATION_FAILED",
            Self::GroupNameNotUnique(_) => "GROUP_NAME_NOT_UNIQUE",
            Self::ForeignKeyViolation(_) => "FOREIGN_KEY_VIOLATION",
            Self::IoError(_) => "IO_ERROR",
            Self::SchemaMigrationFailed(_) => "SCHEMA_MIGRATION_FAILED",
            Self::Internal(_) => "INTERNAL",
        }
    }

    pub fn message(&self) -> String {
        format!("{self}")
    }
}
