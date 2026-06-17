use std::path::{Path, PathBuf};

use domain::DomainError;

pub trait ExportBackend: Send + Sync {
    fn export_markdown(&self, output_path: &Path) -> Result<PathBuf, DomainError>;
    fn export_sqlite(&self, output_path: &Path) -> Result<PathBuf, DomainError>;
    fn import_sqlite(&self, source_path: &Path) -> Result<(), DomainError>;
}

pub struct ExportService {
    backend: std::sync::Arc<dyn ExportBackend>,
}

impl ExportService {
    pub fn new(backend: std::sync::Arc<dyn ExportBackend>) -> Self {
        Self { backend }
    }

    pub fn export_markdown(&self, output_path: &Path) -> Result<PathBuf, DomainError> {
        self.backend.export_markdown(output_path)
    }

    pub fn export_sqlite(&self, output_path: &Path) -> Result<PathBuf, DomainError> {
        self.backend.export_sqlite(output_path)
    }

    pub fn import_sqlite(&self, source_path: &Path) -> Result<(), DomainError> {
        self.backend.import_sqlite(source_path)
    }
}
