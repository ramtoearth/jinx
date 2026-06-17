use std::path::{Path, PathBuf};
use std::sync::Arc;

use domain::{
    DomainError, Note,
    note::{NewNote, NoteRepository, NotePatch},
};

pub struct NoteService {
    repo: Arc<dyn NoteRepository>,
}

impl NoteService {
    pub fn new(repo: Arc<dyn NoteRepository>) -> Self {
        Self { repo }
    }

    pub fn list(&self) -> Result<Vec<Note>, DomainError> {
        self.repo.list_notes()
    }

    pub fn search(&self, query: &str) -> Result<Vec<Note>, DomainError> {
        self.repo.search_notes(query)
    }

    pub fn create(&self, input: NewNote) -> Result<Note, DomainError> {
        self.repo.create_note(input)
    }

    pub fn update(&self, id: i64, patch: NotePatch) -> Result<Note, DomainError> {
        self.repo.update_note(id, patch)
    }

    pub fn delete(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_note(id)
    }

    pub fn export(&self, id: i64, output_path: &Path) -> Result<PathBuf, DomainError> {
        self.repo.export_note(id, output_path)
    }
}
