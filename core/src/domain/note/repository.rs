use std::path::{Path, PathBuf};

use crate::DomainError;
use super::entity::Note;
use super::value_objects::{NewNote, NotePatch};

pub trait NoteRepository: Send + Sync {
    fn list_notes(&self) -> Result<Vec<Note>, DomainError>;
    fn search_notes(&self, query: &str) -> Result<Vec<Note>, DomainError>;
    fn create_note(&self, input: NewNote) -> Result<Note, DomainError>;
    fn update_note(&self, id: i64, patch: NotePatch) -> Result<Note, DomainError>;
    fn delete_note(&self, id: i64) -> Result<(), DomainError>;
    fn export_note(&self, id: i64, output_path: &Path) -> Result<PathBuf, DomainError>;
}
