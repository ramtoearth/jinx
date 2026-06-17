use domain::DomainError;
use rusqlite::{Connection, params};

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
    // Migration 2: seed the Default group
    r#"
    INSERT OR IGNORE INTO groups (name, color) VALUES ('Default', '#6C757D');
    "#,
    // Migration 3: remove unused Default group
    r#"
    DELETE FROM groups
    WHERE name = 'Default'
      AND id NOT IN (SELECT group_id FROM tasks  WHERE group_id IS NOT NULL)
      AND id NOT IN (SELECT group_id FROM events WHERE group_id IS NOT NULL);
    "#,
    // Migration 4: case-insensitive group names
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
    // Migration 5: Google Calendar sync metadata on events (kept for backwards compat)
    r#"
    ALTER TABLE events ADD COLUMN google_event_id TEXT;
    ALTER TABLE events ADD COLUMN google_etag TEXT;
    ALTER TABLE events ADD COLUMN push_pending INTEGER NOT NULL DEFAULT 0;

    CREATE UNIQUE INDEX IF NOT EXISTS idx_events_google_id
        ON events(google_event_id) WHERE google_event_id IS NOT NULL;
    "#,
    // Migration 6: Google Calendar sync metadata on tasks (kept for backwards compat)
    r#"
    ALTER TABLE tasks ADD COLUMN google_event_id TEXT;
    ALTER TABLE tasks ADD COLUMN google_etag TEXT;
    ALTER TABLE tasks ADD COLUMN push_pending INTEGER NOT NULL DEFAULT 0;

    CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_google_id
        ON tasks(google_event_id) WHERE google_event_id IS NOT NULL;
    "#,
    // Migration 7: sync state table (kept for backwards compat)
    r#"
    CREATE TABLE IF NOT EXISTS sync_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        calendar_sync_token TEXT,
        tasks_last_sync TEXT
    );
    INSERT OR IGNORE INTO sync_state (id) VALUES (1);
    "#,
    // Migration 8: Quick Notes table
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

pub(crate) fn apply_migrations(conn: &Connection) -> Result<(), DomainError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);",
    )
    .map_err(|e| DomainError::SchemaMigrationFailed(e.to_string()))?;

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
            .map_err(|e| DomainError::SchemaMigrationFailed(format!("migration {target}: {e}")))?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_version(version) VALUES (?1)",
            params![target],
        )
        .map_err(|e| DomainError::SchemaMigrationFailed(e.to_string()))?;
    }
    Ok(())
}
