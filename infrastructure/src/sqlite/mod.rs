mod migrations;
pub mod task_repo;
pub mod event_repo;
pub mod note_repo;
pub mod group_repo;
pub mod export;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use domain::DomainError;
use rusqlite::Connection;

pub struct SqliteStorage {
    pub(crate) conn: Mutex<Connection>,
}

impl SqliteStorage {
    pub fn open(path: &Path) -> Result<Self, DomainError> {
        let conn = Connection::open(path)
            .map_err(|e| DomainError::Internal(format!("cannot open db: {e}")))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        migrations::apply_migrations(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn in_memory() -> Result<Self, DomainError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| DomainError::Internal(format!("in-memory db: {e}")))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        migrations::apply_migrations(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}

pub fn resolve_db_path() -> Result<PathBuf, DomainError> {
    let dirs = directories::ProjectDirs::from("", "", "jinx").ok_or_else(|| {
        DomainError::Internal("cannot determine user config directory".into())
    })?;
    let config_dir = dirs.config_dir();
    std::fs::create_dir_all(config_dir).map_err(|e| {
        DomainError::Internal(format!("cannot create config dir {config_dir:?}: {e}"))
    })?;
    Ok(config_dir.join("organizer.sqlite3"))
}

pub(crate) fn map_err(e: rusqlite::Error) -> DomainError {
    match e {
        rusqlite::Error::SqliteFailure(ref err, _)
            if err.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            let msg = e.to_string();
            if msg.contains("UNIQUE") && msg.contains("name") {
                DomainError::GroupNameNotUnique(msg)
            } else if msg.contains("FOREIGN KEY") {
                DomainError::ForeignKeyViolation(msg)
            } else {
                DomainError::Internal(msg)
            }
        }
        _ => DomainError::Internal(e.to_string()),
    }
}

pub(crate) fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi, s) = secs_to_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}+00:00")
}

fn secs_to_parts(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    let (y, mo, d) = days_to_ymd(days);
    (y, mo, d, h as u32, m as u32, s as u32)
}

fn days_to_ymd(days: u64) -> (u32, u32, u32) {
    let jdn = days + 2440588;
    let a = jdn + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    (year as u32, month as u32, day as u32)
}
