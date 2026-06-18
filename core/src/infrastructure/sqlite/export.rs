use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::domain::{
    DomainError, HexColor, Priority,
    task::{TaskFilter, TaskRepository},
    calendar::EventRepository,
    group::GroupRepository,
};
use crate::application::export_service::ExportBackend;

use super::SqliteStorage;

impl SqliteStorage {
    pub fn export_markdown(&self, output_path: &Path) -> Result<PathBuf, DomainError> {
        check_writable(output_path)?;

        let tasks = TaskRepository::list_tasks(self, TaskFilter::default())?;
        let events = EventRepository::list_events(self, None, None)?;
        let groups = GroupRepository::list_groups(self)?;

        let mut sorted_tasks = tasks;
        sorted_tasks.sort_by_key(|t| {
            let p = match t.priority {
                Priority::Alta => 1u8,
                Priority::Media => 2,
                Priority::Baja => 3,
            };
            (p, t.deadline.clone().unwrap_or_else(|| "~".to_string()))
        });

        let mut buf = String::with_capacity(4096);

        buf.push_str("# Tareas\n\n");
        buf.push_str("| id | título | prioridad | estado | created_at | deadline | group_id |\n");
        buf.push_str("|---|---|---|---|---|---|---|\n");
        for t in &sorted_tasks {
            buf.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                t.id,
                t.title,
                t.priority.as_str(),
                t.status.as_str(),
                t.created_at,
                t.deadline.as_deref().unwrap_or(""),
                t.group_id.map(|g| g.to_string()).unwrap_or_default(),
            ));
        }

        buf.push_str("\n# Eventos\n\n");
        buf.push_str("| id | título | start_date | start_time | duration_minutes | group_id |\n");
        buf.push_str("|---|---|---|---|---|---|\n");
        let mut sorted_events = events;
        sorted_events.sort_by_key(|e| (e.start_date.clone(), e.start_time.clone()));
        for e in &sorted_events {
            buf.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                e.id,
                e.title,
                e.start_date,
                e.start_time,
                e.duration_minutes.map(|d| d.to_string()).unwrap_or_default(),
                e.group_id.map(|g| g.to_string()).unwrap_or_default(),
            ));
        }

        buf.push_str("\n# Grupos\n\n");
        buf.push_str("| id | nombre | color |\n");
        buf.push_str("|---|---|---|\n");
        for g in &groups {
            buf.push_str(&format!("| {} | {} | {} |\n", g.id, g.name, g.color));
        }

        write_file(output_path, buf.as_bytes())?;
        Ok(output_path.to_path_buf())
    }

    pub fn export_sqlite(&self, output_path: &Path) -> Result<PathBuf, DomainError> {
        check_writable(output_path)?;

        let groups = GroupRepository::list_groups(self)?;
        let tasks = TaskRepository::list_tasks(self, TaskFilter::default())?;
        let events = EventRepository::list_events(self, None, None)?;

        let out_conn = rusqlite::Connection::open(output_path)
            .map_err(|e| DomainError::IoError(format!("cannot create export db: {e}")))?;

        out_conn
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(|e| DomainError::Internal(e.to_string()))?;

        out_conn
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);
                CREATE TABLE IF NOT EXISTS groups (
                    id    INTEGER PRIMARY KEY AUTOINCREMENT,
                    name  TEXT    NOT NULL UNIQUE,
                    color TEXT    NOT NULL
                );
                CREATE TABLE IF NOT EXISTS tasks (
                    id         INTEGER PRIMARY KEY AUTOINCREMENT,
                    title      TEXT    NOT NULL,
                    priority   TEXT    NOT NULL CHECK (priority IN ('alta','media','baja')),
                    status     TEXT    NOT NULL CHECK (status IN ('pendiente','completada','cancelada')),
                    created_at TEXT    NOT NULL,
                    deadline   TEXT,
                    group_id   INTEGER,
                    FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE SET NULL
                );
                CREATE TABLE IF NOT EXISTS events (
                    id               INTEGER PRIMARY KEY AUTOINCREMENT,
                    title            TEXT    NOT NULL,
                    start_date       TEXT    NOT NULL,
                    start_time       TEXT    NOT NULL,
                    duration_minutes INTEGER,
                    group_id         INTEGER,
                    FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE SET NULL
                );
                CREATE INDEX IF NOT EXISTS idx_tasks_status_priority ON tasks(status, priority);
                CREATE INDEX IF NOT EXISTS idx_tasks_deadline        ON tasks(deadline);
                CREATE INDEX IF NOT EXISTS idx_tasks_group           ON tasks(group_id);
                CREATE INDEX IF NOT EXISTS idx_events_start_date     ON events(start_date);
                CREATE INDEX IF NOT EXISTS idx_events_group          ON events(group_id);
                INSERT OR REPLACE INTO schema_version(version) VALUES (1);
                "#,
            )
            .map_err(|e| DomainError::Internal(format!("export schema: {e}")))?;

        let tx = out_conn
            .unchecked_transaction()
            .map_err(|e| DomainError::Internal(e.to_string()))?;

        for g in &groups {
            tx.execute(
                "INSERT INTO groups (id, name, color) VALUES (?1, ?2, ?3)",
                rusqlite::params![g.id, g.name, g.color.as_str()],
            )
            .map_err(|e| DomainError::Internal(format!("export group {}: {e}", g.id)))?;
        }

        for t in &tasks {
            tx.execute(
                "INSERT INTO tasks (id, title, priority, status, created_at, deadline, group_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    t.id,
                    t.title,
                    t.priority.as_str(),
                    t.status.as_str(),
                    t.created_at,
                    t.deadline,
                    t.group_id,
                ],
            )
            .map_err(|e| DomainError::Internal(format!("export task {}: {e}", t.id)))?;
        }

        for e in &events {
            tx.execute(
                "INSERT INTO events (id, title, start_date, start_time, duration_minutes, group_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    e.id,
                    e.title,
                    e.start_date,
                    e.start_time,
                    e.duration_minutes,
                    e.group_id,
                ],
            )
            .map_err(|e| DomainError::Internal(format!("export event: {e}")))?;
        }

        tx.commit()
            .map_err(|e| DomainError::Internal(format!("export commit: {e}")))?;

        Ok(output_path.to_path_buf())
    }

    pub fn import_sqlite(&self, source_path: &Path) -> Result<(), DomainError> {
        if !source_path.exists() {
            return Err(DomainError::IoError(format!(
                "source file not found: {source_path:?}"
            )));
        }

        let src_conn = rusqlite::Connection::open(source_path)
            .map_err(|e| DomainError::IoError(format!("cannot open source db: {e}")))?;

        let groups: Vec<(i64, String, String)> = {
            let mut stmt = src_conn
                .prepare("SELECT id, name, color FROM groups ORDER BY id ASC")
                .map_err(|e| DomainError::IoError(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(|e| DomainError::IoError(e.to_string()))?;
            let mut gs = vec![];
            for row in rows {
                gs.push(row.map_err(|e| DomainError::IoError(e.to_string()))?);
            }
            gs
        };

        type TaskRow = (i64, String, String, String, String, Option<String>, Option<i64>);
        type EventRow = (i64, String, String, String, Option<u32>, Option<i64>);

        let tasks_raw: Vec<TaskRow> = {
            let mut stmt = src_conn
                .prepare(
                    "SELECT id, title, priority, status, created_at, deadline, group_id
                     FROM tasks ORDER BY id ASC",
                )
                .map_err(|e| DomainError::IoError(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                })
                .map_err(|e| DomainError::IoError(e.to_string()))?;
            let mut ts = vec![];
            for row in rows {
                ts.push(row.map_err(|e| DomainError::IoError(e.to_string()))?);
            }
            ts
        };

        let events_raw: Vec<EventRow> = {
            let mut stmt = src_conn
                .prepare(
                    "SELECT id, title, start_date, start_time, duration_minutes, group_id
                     FROM events ORDER BY id ASC",
                )
                .map_err(|e| DomainError::IoError(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(|e| DomainError::IoError(e.to_string()))?;
            let mut es = vec![];
            for row in rows {
                es.push(row.map_err(|e| DomainError::IoError(e.to_string()))?);
            }
            es
        };

        use crate::domain::group::NewGroup as DomainNewGroup;
        use crate::domain::task::NewTask as DomainNewTask;
        use crate::domain::calendar::NewEvent as DomainNewEvent;

        for (_, name, color) in &groups {
            let hex = HexColor::new(color.clone())
                .map_err(DomainError::ValidationFailed)?;
            match GroupRepository::create_group(self, DomainNewGroup { name: name.clone(), color: hex }) {
                Ok(_) | Err(DomainError::GroupNameNotUnique(_)) => {}
                Err(e) => return Err(e),
            }
        }

        for (_, title, priority_str, _status, _created_at, deadline, group_id) in &tasks_raw {
            let priority: crate::domain::Priority = priority_str.parse().map_err(DomainError::ValidationFailed)?;
            TaskRepository::create_task(self, DomainNewTask {
                title: title.clone(),
                priority: Some(priority),
                deadline: deadline.clone(),
                group_id: *group_id,
            })?;
        }

        for (_, title, start_date, start_time, duration_minutes, group_id) in &events_raw {
            EventRepository::create_event(self, DomainNewEvent {
                title: title.clone(),
                start_date: start_date.clone(),
                start_time: start_time.clone(),
                duration_minutes: *duration_minutes,
                group_id: *group_id,
            })?;
        }

        Ok(())
    }
}

impl ExportBackend for SqliteStorage {
    fn export_markdown(&self, output_path: &Path) -> Result<PathBuf, DomainError> {
        self.export_markdown(output_path)
    }

    fn export_sqlite(&self, output_path: &Path) -> Result<PathBuf, DomainError> {
        self.export_sqlite(output_path)
    }

    fn import_sqlite(&self, source_path: &Path) -> Result<(), DomainError> {
        self.import_sqlite(source_path)
    }
}

fn check_writable(path: &Path) -> Result<(), DomainError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(DomainError::IoError(format!(
                "parent directory does not exist: {parent:?}"
            )));
        }
    }
    match std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path) {
        Ok(_) => Ok(()),
        Err(e) => Err(DomainError::IoError(format!(
            "path not writable: {path:?}: {e}"
        ))),
    }
}

fn write_file(path: &Path, data: &[u8]) -> Result<(), DomainError> {
    let mut file = std::fs::File::create(path)
        .map_err(|e| DomainError::IoError(format!("cannot create file {path:?}: {e}")))?;
    file.write_all(data)
        .map_err(|e| DomainError::IoError(format!("cannot write to {path:?}: {e}")))?;
    Ok(())
}
