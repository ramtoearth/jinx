//! Resolves the platform-specific path of the SQLite database file.
//!
//! Requisitos 7.1, 9.2

use directories::ProjectDirs;
use std::path::PathBuf;

use crate::StorageError;

/// Returns the path to `organizer.sqlite3` inside the user's config directory,
/// creating intermediate directories if they do not exist.
///
/// | Platform | Base directory                                                                              |
/// |----------|---------------------------------------------------------------------------------------------|
/// | Linux    | `$XDG_CONFIG_HOME` or `$HOME/.config/jinx/`                                                |
/// | macOS    | `$HOME/Library/Application Support/jinx/`                                                  |
/// | Windows  | `%APPDATA%\jinx\`                                                                           |
pub fn resolve_db_path() -> Result<PathBuf, StorageError> {
    let dirs = ProjectDirs::from("", "", "jinx").ok_or_else(|| {
        StorageError::Internal("cannot determine user config directory".into())
    })?;
    let config_dir = dirs.config_dir();
    std::fs::create_dir_all(config_dir).map_err(|e| {
        StorageError::Internal(format!("cannot create config dir {config_dir:?}: {e}"))
    })?;
    Ok(config_dir.join("organizer.sqlite3"))
}
