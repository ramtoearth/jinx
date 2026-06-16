//! `storage` crate — SQLite-backed Almacén for the Terminal Day Organizer.
//!
//! Exposes:
//! - [`resolve_db_path`]: computes the platform-specific SQLite path.
//! - [`HexColor`]: validated `#RRGGBB` colour string.
//! - Domain structs: [`Group`], [`Task`], [`Event`].
//! - Input/patch types: [`NewTask`], [`TaskPatch`], [`NewEvent`], [`EventPatch`],
//!   [`NewGroup`], [`GroupPatch`], [`TaskFilter`], [`GroupsSnapshot`].
//! - [`Storage`] trait + [`StorageError`] enum.
//! - [`SqliteStorage`]: the concrete SQLite implementation.
//! - [`Exporter`]: Markdown and SQLite export/import.

pub mod db;
pub mod error;
pub mod export;
pub mod models;
pub mod path;

pub use db::SqliteStorage;
pub use error::StorageError;
pub use export::Exporter;
pub use models::{
    Event, EventPatch, Group, GroupPatch, GroupsSnapshot, HexColor, NewEvent, NewGroup, NewNote,
    NewTask, Note, NotePatch, Priority, Task, TaskFilter, TaskPatch, TaskStatus,
};
pub use path::resolve_db_path;

use std::path::PathBuf;

/// Unified `Storage` trait — every operation that the TUI or the Agent may
/// invoke on the Almacén.
pub trait Storage {
    fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>, StorageError>;
    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, StorageError>;
    fn create_task(&self, input: NewTask) -> Result<Task, StorageError>;
    fn update_task(&self, id: i64, patch: TaskPatch) -> Result<Task, StorageError>;
    fn complete_task(&self, id: i64) -> Result<Task, StorageError>;
    fn delete_task(&self, id: i64) -> Result<(), StorageError>;

    fn list_events(&self, from_date: Option<&str>, to_date: Option<&str>)
        -> Result<Vec<Event>, StorageError>;
    fn create_event(&self, input: NewEvent) -> Result<Event, StorageError>;
    fn update_event(&self, id: i64, patch: EventPatch) -> Result<Event, StorageError>;
    fn delete_event(&self, id: i64) -> Result<(), StorageError>;

    fn list_groups(&self) -> Result<Vec<Group>, StorageError>;
    fn find_group_by_name(&self, name: &str) -> Result<Option<Group>, StorageError>;
    fn create_group(&self, input: NewGroup) -> Result<Group, StorageError>;
    fn rename_group(&self, id: i64, name: String) -> Result<Group, StorageError>;
    fn recolor_group(&self, id: i64, color: HexColor) -> Result<Group, StorageError>;
    fn delete_group(&self, id: i64) -> Result<(), StorageError>;

    fn snapshot_for_inference(&self) -> Result<GroupsSnapshot, StorageError>;

    fn export_markdown(&self, output_path: &std::path::Path) -> Result<PathBuf, StorageError>;
    fn export_sqlite(&self, output_path: &std::path::Path) -> Result<PathBuf, StorageError>;
    fn import_sqlite(&self, source_path: &std::path::Path) -> Result<(), StorageError>;

    fn list_notes(&self) -> Result<Vec<Note>, StorageError>;
    fn search_notes(&self, query: &str) -> Result<Vec<Note>, StorageError>;
    fn create_note(&self, input: NewNote) -> Result<Note, StorageError>;
    fn update_note(&self, id: i64, patch: NotePatch) -> Result<Note, StorageError>;
    fn delete_note(&self, id: i64) -> Result<(), StorageError>;

    fn mark_push_pending(&self, event_id: i64) -> Result<(), StorageError>;
    fn mark_task_push_pending(&self, task_id: i64) -> Result<(), StorageError>;
    fn mark_all_push_pending(&self) -> Result<(), StorageError>;
    fn get_google_event_id(&self, event_id: i64) -> Result<Option<String>, StorageError>;
    fn get_task_google_event_id(&self, task_id: i64) -> Result<Option<String>, StorageError>;
}
