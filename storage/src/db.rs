//! `SqliteStorage` — concrete SQLite implementation of the `Storage` trait.

use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

use crate::{
    Storage, StorageError,
    models::{
        Event, EventPatch, Group, GroupInfo, GroupsSnapshot, HexColor, NewEvent, NewGroup,
        NewNote, NewTask, Note, NotePatch, Priority, Task, TaskFilter, TaskPatch, TaskStatus,
    },
};

// ---------------------------------------------------------------------------
// Schema migrations
// ---------------------------------------------------------------------------

/// Each entry is applied once, in order, when the stored version is lower.
static MIGRATIONS: &[&str] = &[
    // Migration 1: initial schema
    r#"
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
    "#,
    // Migration 2: seed the Default group so the agent always has a fallback
    r#"
    INSERT OR IGNORE INTO groups (name, color) VALUES ('Default', '#6C757D');
    "#,
    // Migration 3: remove the Default group seeded by V2 if no data references it
    r#"
    DELETE FROM groups
    WHERE name = 'Default'
      AND id NOT IN (SELECT group_id FROM tasks  WHERE group_id IS NOT NULL)
      AND id NOT IN (SELECT group_id FROM events WHERE group_id IS NOT NULL);
    "#,
    // Migration 4: enforce case-insensitive uniqueness on group names.
    // Remove duplicates keeping the lowest id, then recreate table with COLLATE NOCASE.
    r#"
    DELETE FROM groups WHERE id NOT IN (
        SELECT MIN(id) FROM groups GROUP BY LOWER(name)
    );
    CREATE TABLE groups_new (
        id    INTEGER PRIMARY KEY AUTOINCREMENT,
        name  TEXT    NOT NULL UNIQUE COLLATE NOCASE,
        color TEXT    NOT NULL
    );
    INSERT INTO groups_new (id, name, color) SELECT id, name, color FROM groups;
    DROP TABLE groups;
    ALTER TABLE groups_new RENAME TO groups;
    "#,
    // Migration 5: Google Calendar sync metadata on events.
    r#"
    ALTER TABLE events ADD COLUMN google_event_id TEXT;
    ALTER TABLE events ADD COLUMN google_etag TEXT;
    ALTER TABLE events ADD COLUMN push_pending INTEGER NOT NULL DEFAULT 0;

    CREATE UNIQUE INDEX IF NOT EXISTS idx_events_google_id
        ON events(google_event_id) WHERE google_event_id IS NOT NULL;
    "#,
    // Migration 6: Google Calendar sync metadata on tasks.
    r#"
    ALTER TABLE tasks ADD COLUMN google_event_id TEXT;
    ALTER TABLE tasks ADD COLUMN google_etag TEXT;
    ALTER TABLE tasks ADD COLUMN push_pending INTEGER NOT NULL DEFAULT 0;

    CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_google_id
        ON tasks(google_event_id) WHERE google_event_id IS NOT NULL;
    "#,
    // Migration 7: sync state for incremental pull from Google.
    r#"
    CREATE TABLE IF NOT EXISTS sync_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        calendar_sync_token TEXT,
        tasks_last_sync TEXT
    );
    INSERT OR IGNORE INTO sync_state (id) VALUES (1);
    "#,
    // Migration 8: Quick Notes table.
    r#"
    CREATE TABLE IF NOT EXISTS notes (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        title      TEXT    NOT NULL,
        body       TEXT    NOT NULL DEFAULT '',
        created_at TEXT    NOT NULL,
        updated_at TEXT    NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at DESC);
    "#,
];

fn apply_migrations(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);",
    )
    .map_err(|e| StorageError::SchemaMigrationFailed(e.to_string()))?;

    let version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    for (i, migration) in MIGRATIONS.iter().enumerate() {
        let target = (i + 1) as i64;
        if version >= target {
            continue;
        }
        conn.execute_batch(migration)
            .map_err(|e| StorageError::SchemaMigrationFailed(format!("migration {target}: {e}")))?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_version(version) VALUES (?1)",
            params![target],
        )
        .map_err(|e| StorageError::SchemaMigrationFailed(e.to_string()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SqliteStorage
// ---------------------------------------------------------------------------

pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    /// Open (or create) the database at `path` and apply pending migrations.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(path)
            .map_err(|e| StorageError::Internal(format!("cannot open db: {e}")))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        apply_migrations(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Open an in-memory database (useful for tests).
    pub fn in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| StorageError::Internal(format!("in-memory db: {e}")))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| StorageError::Internal(e.to_string()))?;
        apply_migrations(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let priority_str: String = row.get(2)?;
    let status_str: String = row.get(3)?;
    let push_pending_int: i32 = row.get(8)?;
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        priority: priority_str.parse().unwrap_or(Priority::Media),
        status: status_str.parse().unwrap_or(TaskStatus::Pendiente),
        created_at: row.get(4)?,
        deadline: row.get(5)?,
        group_id: row.get(6)?,
        google_event_id: row.get(7)?,
        push_pending: push_pending_int != 0,
    })
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let push_pending_int: i32 = row.get(7)?;
    Ok(Event {
        id: row.get(0)?,
        title: row.get(1)?,
        start_date: row.get(2)?,
        start_time: row.get(3)?,
        duration_minutes: row.get(4)?,
        group_id: row.get(5)?,
        google_event_id: row.get(6)?,
        push_pending: push_pending_int != 0,
    })
}

fn map_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<Group> {
    let color_str: String = row.get(2)?;
    Ok(Group {
        id: row.get(0)?,
        name: row.get(1)?,
        color: HexColor::new(color_str).unwrap_or_else(|_| HexColor::new("#808080").unwrap()),
    })
}

fn map_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

// ---------------------------------------------------------------------------
// Helper to get current UTC timestamp
// ---------------------------------------------------------------------------

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as ISO 8601 with UTC offset
    let (y, mo, d, h, mi, s) = secs_to_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}+00:00")
}

fn secs_to_parts(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Gregorian calendar computation
    let (y, mo, d) = days_to_ymd(days);
    (y, mo, d, h as u32, m as u32, s as u32)
}

fn days_to_ymd(days: u64) -> (u32, u32, u32) {
    // Julian Day Number epoch offset from Unix epoch
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

// ---------------------------------------------------------------------------
// Storage trait implementation
// ---------------------------------------------------------------------------

impl Storage for SqliteStorage {
    // -----------------------------------------------------------------------
    // Tasks
    // -----------------------------------------------------------------------

    fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>, StorageError> {
        let conn = self.conn.lock().unwrap();
        // Build a dynamic WHERE clause
        let mut clauses: Vec<String> = vec![];
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![];

        if let Some(status) = filter.status {
            clauses.push("status = ?".to_string());
            binds.push(Box::new(status.as_str().to_string()));
        }
        match filter.group_id {
            Some(Some(gid)) => {
                clauses.push("group_id = ?".to_string());
                binds.push(Box::new(gid));
            }
            Some(None) => {
                clauses.push("group_id IS NULL".to_string());
            }
            None => {}
        }
        if filter.no_deadline {
            clauses.push("deadline IS NULL".to_string());
        } else {
            if let Some(from) = filter.from_date {
                clauses.push("deadline >= ?".to_string());
                binds.push(Box::new(from));
            }
            if let Some(to) = filter.to_date {
                let effective = if to.len() == 10 && !to.contains('T') {
                    format!("{}T23:59:59+23:59", to)
                } else {
                    to
                };
                clauses.push("deadline <= ?".to_string());
                binds.push(Box::new(effective));
            }
        }

        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        // Order: alta < media < baja (alphabetical descends, use CASE)
        let sql = format!(
            "SELECT id, title, priority, status, created_at, deadline, group_id, google_event_id, push_pending
             FROM tasks
             {where_clause}
             ORDER BY
               CASE priority WHEN 'alta' THEN 1 WHEN 'media' THEN 2 ELSE 3 END,
               CASE WHEN deadline IS NULL THEN 1 ELSE 0 END,
               deadline ASC"
        );

        let mut stmt = conn.prepare(&sql).map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
                map_task,
            )
            .map_err(StorageError::from)?;

        let mut tasks = vec![];
        for row in rows {
            tasks.push(row.map_err(StorageError::from)?);
        }
        Ok(tasks)
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT id, title, priority, status, created_at, deadline, group_id, google_event_id, push_pending
                 FROM tasks
                 WHERE title LIKE ?1
                 ORDER BY
                   CASE priority WHEN 'alta' THEN 1 WHEN 'media' THEN 2 ELSE 3 END,
                   CASE WHEN deadline IS NULL THEN 1 ELSE 0 END,
                   deadline ASC",
            )
            .map_err(StorageError::from)?;
        let rows = stmt.query_map(params![pattern], map_task).map_err(StorageError::from)?;
        let mut tasks = vec![];
        for row in rows {
            tasks.push(row.map_err(StorageError::from)?);
        }
        Ok(tasks)
    }

    fn create_task(&self, input: NewTask) -> Result<Task, StorageError> {
        let conn = self.conn.lock().unwrap();
        let priority = input.priority.unwrap_or(Priority::Media);
        let created_at = now_iso();

        if let Some(gid) = input.group_id {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM groups WHERE id = ?1",
                    params![gid],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !exists {
                return Err(StorageError::ForeignKeyViolation(format!(
                    "group_id {gid} does not exist"
                )));
            }
        }

        conn.execute(
            "INSERT INTO tasks (title, priority, status, created_at, deadline, group_id)
             VALUES (?1, ?2, 'pendiente', ?3, ?4, ?5)",
            params![
                input.title,
                priority.as_str(),
                created_at,
                input.deadline,
                input.group_id,
            ],
        )
        .map_err(StorageError::from)?;

        let id = conn.last_insert_rowid();
        self.get_task_by_id(&conn, id)
    }

    fn update_task(&self, id: i64, patch: TaskPatch) -> Result<Task, StorageError> {
        let conn = self.conn.lock().unwrap();
        let existing = self.get_task_by_id(&conn, id)?;

        let title = patch.title.unwrap_or(existing.title);
        let priority = patch.priority.unwrap_or(existing.priority);
        let status = patch.status.unwrap_or(existing.status);
        let deadline = match patch.deadline {
            Some(d) => d,
            None => existing.deadline,
        };
        let group_id = match patch.group_id {
            Some(g) => g,
            None => existing.group_id,
        };

        if let Some(gid) = group_id {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM groups WHERE id = ?1",
                    params![gid],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !exists {
                return Err(StorageError::ForeignKeyViolation(format!(
                    "group_id {gid} does not exist"
                )));
            }
        }

        conn.execute(
            "UPDATE tasks SET title=?1, priority=?2, status=?3, deadline=?4, group_id=?5
             WHERE id=?6",
            params![title, priority.as_str(), status.as_str(), deadline, group_id, id],
        )
        .map_err(StorageError::from)?;

        self.get_task_by_id(&conn, id)
    }

    fn complete_task(&self, id: i64) -> Result<Task, StorageError> {
        let conn = self.conn.lock().unwrap();
        // Ensure it exists first
        self.get_task_by_id(&conn, id)?;
        conn.execute(
            "UPDATE tasks SET status='completada' WHERE id=?1",
            params![id],
        )
        .map_err(StorageError::from)?;
        self.get_task_by_id(&conn, id)
    }

    fn delete_task(&self, id: i64) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        self.get_task_by_id(&conn, id)?;
        conn.execute("DELETE FROM tasks WHERE id=?1", params![id])
            .map_err(StorageError::from)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    fn list_events(
        &self,
        from_date: Option<&str>,
        to_date: Option<&str>,
    ) -> Result<Vec<Event>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut clauses: Vec<String> = vec![];
        let mut binds: Vec<String> = vec![];

        if let Some(from) = from_date {
            clauses.push("start_date >= ?".to_string());
            binds.push(from.to_string());
        }
        if let Some(to) = to_date {
            clauses.push("start_date <= ?".to_string());
            binds.push(to.to_string());
        }

        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        let sql = format!(
            "SELECT id, title, start_date, start_time, duration_minutes, group_id, google_event_id, push_pending
             FROM events
             {where_clause}
             ORDER BY start_date ASC, start_time ASC"
        );

        let mut stmt = conn.prepare(&sql).map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter()),
                map_event,
            )
            .map_err(StorageError::from)?;

        let mut events = vec![];
        for row in rows {
            events.push(row.map_err(StorageError::from)?);
        }
        Ok(events)
    }

    fn create_event(&self, input: NewEvent) -> Result<Event, StorageError> {
        let conn = self.conn.lock().unwrap();

        if let Some(gid) = input.group_id {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM groups WHERE id = ?1",
                    params![gid],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !exists {
                return Err(StorageError::ForeignKeyViolation(format!(
                    "group_id {gid} does not exist"
                )));
            }
        }

        conn.execute(
            "INSERT INTO events (title, start_date, start_time, duration_minutes, group_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                input.title,
                input.start_date,
                input.start_time,
                input.duration_minutes,
                input.group_id,
            ],
        )
        .map_err(StorageError::from)?;

        let id = conn.last_insert_rowid();
        self.get_event_by_id(&conn, id)
    }

    fn update_event(&self, id: i64, patch: EventPatch) -> Result<Event, StorageError> {
        let conn = self.conn.lock().unwrap();
        let existing = self.get_event_by_id(&conn, id)?;

        let title = patch.title.unwrap_or(existing.title);
        let start_date = patch.start_date.unwrap_or(existing.start_date);
        let start_time = patch.start_time.unwrap_or(existing.start_time);
        let duration_minutes = match patch.duration_minutes {
            Some(d) => d,
            None => existing.duration_minutes,
        };
        let group_id = match patch.group_id {
            Some(g) => g,
            None => existing.group_id,
        };

        if let Some(gid) = group_id {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM groups WHERE id = ?1",
                    params![gid],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !exists {
                return Err(StorageError::ForeignKeyViolation(format!(
                    "group_id {gid} does not exist"
                )));
            }
        }

        conn.execute(
            "UPDATE events SET title=?1, start_date=?2, start_time=?3,
             duration_minutes=?4, group_id=?5 WHERE id=?6",
            params![title, start_date, start_time, duration_minutes, group_id, id],
        )
        .map_err(StorageError::from)?;

        self.get_event_by_id(&conn, id)
    }

    fn delete_event(&self, id: i64) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        self.get_event_by_id(&conn, id)?;
        conn.execute("DELETE FROM events WHERE id=?1", params![id])
            .map_err(StorageError::from)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Groups
    // -----------------------------------------------------------------------

    fn list_groups(&self) -> Result<Vec<Group>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, name, color FROM groups ORDER BY id ASC")
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map([], map_group)
            .map_err(StorageError::from)?;
        let mut groups = vec![];
        for row in rows {
            groups.push(row.map_err(StorageError::from)?);
        }
        Ok(groups)
    }

    fn find_group_by_name(&self, name: &str) -> Result<Option<Group>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT id, name, color FROM groups WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |row| {
                    Ok(Group {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: HexColor::new(
                            &row.get::<_, String>(2)?
                        ).unwrap_or_else(|_| HexColor::new("#000000").unwrap()),
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)?;
        Ok(result)
    }

    fn create_group(&self, input: NewGroup) -> Result<Group, StorageError> {
        let conn = self.conn.lock().unwrap();
        match conn.execute(
            "INSERT INTO groups (name, color) VALUES (?1, ?2)",
            params![input.name, input.color.as_str()],
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(ref e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(StorageError::GroupNameNotUnique(format!(
                    "Ya existe un Grupo llamado '{}'",
                    input.name
                )));
            }
            Err(e) => return Err(StorageError::from(e)),
        }
        let id = conn.last_insert_rowid();
        self.get_group_by_id(&conn, id)
    }

    fn rename_group(&self, id: i64, name: String) -> Result<Group, StorageError> {
        let conn = self.conn.lock().unwrap();
        self.get_group_by_id(&conn, id)?;
        match conn.execute(
            "UPDATE groups SET name=?1 WHERE id=?2",
            params![name, id],
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(ref e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(StorageError::GroupNameNotUnique(format!(
                    "Ya existe un Grupo llamado '{name}'"
                )));
            }
            Err(e) => return Err(StorageError::from(e)),
        }
        self.get_group_by_id(&conn, id)
    }

    fn recolor_group(&self, id: i64, color: HexColor) -> Result<Group, StorageError> {
        let conn = self.conn.lock().unwrap();
        self.get_group_by_id(&conn, id)?;
        conn.execute(
            "UPDATE groups SET color=?1 WHERE id=?2",
            params![color.as_str(), id],
        )
        .map_err(StorageError::from)?;
        self.get_group_by_id(&conn, id)
    }

    fn delete_group(&self, id: i64) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        self.get_group_by_id(&conn, id)?;
        // ON DELETE SET NULL handles nullifying group_id on tasks/events
        conn.execute("DELETE FROM groups WHERE id=?1", params![id])
            .map_err(StorageError::from)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Snapshot for group inference
    // -----------------------------------------------------------------------

    fn snapshot_for_inference(&self) -> Result<GroupsSnapshot, StorageError> {
        let conn = self.conn.lock().unwrap();
        let groups = {
            let mut stmt = conn
                .prepare("SELECT id, name, color FROM groups ORDER BY id ASC")
                .map_err(StorageError::from)?;
            let rows = stmt
                .query_map([], map_group)
                .map_err(StorageError::from)?;
            let mut gs = vec![];
            for row in rows {
                gs.push(row.map_err(StorageError::from)?);
            }
            gs
        };

        let mut snapshot = vec![];
        for group in &groups {
            let mut titles: Vec<String> = vec![];
            {
                let mut stmt = conn
                    .prepare("SELECT title FROM tasks WHERE group_id=?1")
                    .map_err(StorageError::from)?;
                let rows = stmt
                    .query_map(params![group.id], |r| r.get(0))
                    .map_err(StorageError::from)?;
                for row in rows {
                    titles.push(row.map_err(StorageError::from)?);
                }
            }
            {
                let mut stmt = conn
                    .prepare("SELECT title FROM events WHERE group_id=?1")
                    .map_err(StorageError::from)?;
                let rows = stmt
                    .query_map(params![group.id], |r| r.get(0))
                    .map_err(StorageError::from)?;
                for row in rows {
                    titles.push(row.map_err(StorageError::from)?);
                }
            }
            snapshot.push(GroupInfo {
                id: group.id,
                name: group.name.clone(),
                member_titles: titles,
            });
        }
        Ok(snapshot)
    }

    // -----------------------------------------------------------------------
    // Export / import — delegated to Exporter
    // -----------------------------------------------------------------------

    fn export_markdown(
        &self,
        output_path: &std::path::Path,
    ) -> Result<std::path::PathBuf, StorageError> {
        crate::export::export_markdown(self, output_path)
    }

    fn export_sqlite(
        &self,
        output_path: &std::path::Path,
    ) -> Result<std::path::PathBuf, StorageError> {
        crate::export::export_sqlite(self, output_path)
    }

    fn import_sqlite(&self, source_path: &std::path::Path) -> Result<(), StorageError> {
        crate::export::import_sqlite(self, source_path)
    }

    fn mark_push_pending(&self, event_id: i64) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE events SET push_pending = 1 WHERE id = ?1",
            params![event_id],
        )
        .map_err(StorageError::from)?;
        Ok(())
    }

    fn mark_task_push_pending(&self, task_id: i64) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET push_pending = 1 WHERE id = ?1",
            params![task_id],
        )
        .map_err(StorageError::from)?;
        Ok(())
    }

    fn mark_all_push_pending(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "UPDATE events SET push_pending = 1; UPDATE tasks SET push_pending = 1;"
        )
        .map_err(|e| StorageError::Internal(e.to_string()))?;
        Ok(())
    }

    fn get_google_event_id(&self, event_id: i64) -> Result<Option<String>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let result: Option<String> = conn
            .query_row(
                "SELECT google_event_id FROM events WHERE id = ?1",
                params![event_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)?
            .flatten();
        Ok(result)
    }

    fn get_task_google_event_id(&self, task_id: i64) -> Result<Option<String>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let result: Option<String> = conn
            .query_row(
                "SELECT google_event_id FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)?
            .flatten();
        Ok(result)
    }

    // -------------------------------------------------------------------
    // Notes
    // -------------------------------------------------------------------

    fn list_notes(&self) -> Result<Vec<Note>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, title, body, created_at, updated_at FROM notes ORDER BY updated_at DESC")
            .map_err(StorageError::from)?;
        let rows = stmt.query_map([], map_note).map_err(StorageError::from)?;
        let mut notes = vec![];
        for row in rows {
            notes.push(row.map_err(StorageError::from)?);
        }
        Ok(notes)
    }

    fn search_notes(&self, query: &str) -> Result<Vec<Note>, StorageError> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT id, title, body, created_at, updated_at FROM notes
                 WHERE title LIKE ?1 OR body LIKE ?1
                 ORDER BY updated_at DESC",
            )
            .map_err(StorageError::from)?;
        let rows = stmt.query_map(params![pattern], map_note).map_err(StorageError::from)?;
        let mut notes = vec![];
        for row in rows {
            notes.push(row.map_err(StorageError::from)?);
        }
        Ok(notes)
    }

    fn create_note(&self, input: NewNote) -> Result<Note, StorageError> {
        let conn = self.conn.lock().unwrap();
        let ts = now_iso();
        conn.execute(
            "INSERT INTO notes (title, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![input.title, input.body, ts, ts],
        )
        .map_err(StorageError::from)?;
        let id = conn.last_insert_rowid();
        self.get_note_by_id(&conn, id)
    }

    fn update_note(&self, id: i64, patch: NotePatch) -> Result<Note, StorageError> {
        let conn = self.conn.lock().unwrap();
        let existing = self.get_note_by_id(&conn, id)?;
        let title = patch.title.unwrap_or(existing.title);
        let body = patch.body.unwrap_or(existing.body);
        let updated_at = now_iso();
        conn.execute(
            "UPDATE notes SET title=?1, body=?2, updated_at=?3 WHERE id=?4",
            params![title, body, updated_at, id],
        )
        .map_err(StorageError::from)?;
        self.get_note_by_id(&conn, id)
    }

    fn delete_note(&self, id: i64) -> Result<(), StorageError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute("DELETE FROM notes WHERE id=?1", params![id])
            .map_err(StorageError::from)?;
        if affected == 0 {
            return Err(StorageError::NotFound(format!("Note {id} not found")));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

impl SqliteStorage {
    fn get_task_by_id(&self, conn: &Connection, id: i64) -> Result<Task, StorageError> {
        conn.query_row(
            "SELECT id, title, priority, status, created_at, deadline, group_id, google_event_id, push_pending
             FROM tasks WHERE id=?1",
            params![id],
            map_task,
        )
        .optional()
        .map_err(StorageError::from)?
        .ok_or_else(|| StorageError::NotFound(format!("Task {id} not found")))
    }

    fn get_event_by_id(&self, conn: &Connection, id: i64) -> Result<Event, StorageError> {
        conn.query_row(
            "SELECT id, title, start_date, start_time, duration_minutes, group_id, google_event_id, push_pending
             FROM events WHERE id=?1",
            params![id],
            map_event,
        )
        .optional()
        .map_err(StorageError::from)?
        .ok_or_else(|| StorageError::NotFound(format!("Event {id} not found")))
    }

    fn get_group_by_id(&self, conn: &Connection, id: i64) -> Result<Group, StorageError> {
        conn.query_row(
            "SELECT id, name, color FROM groups WHERE id=?1",
            params![id],
            map_group,
        )
        .optional()
        .map_err(StorageError::from)?
        .ok_or_else(|| StorageError::NotFound(format!("Group {id} not found")))
    }

    fn get_note_by_id(&self, conn: &Connection, id: i64) -> Result<Note, StorageError> {
        conn.query_row(
            "SELECT id, title, body, created_at, updated_at FROM notes WHERE id=?1",
            params![id],
            map_note,
        )
        .optional()
        .map_err(StorageError::from)?
        .ok_or_else(|| StorageError::NotFound(format!("Note {id} not found")))
    }
}
