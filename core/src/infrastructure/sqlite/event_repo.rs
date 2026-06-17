use crate::domain::{
    DomainError, Event,
    calendar::{EventPatch, EventRepository, NewEvent},
};
use rusqlite::{Connection, params};

use super::{SqliteStorage, map_err};

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: row.get(0)?,
        title: row.get(1)?,
        start_date: row.get(2)?,
        start_time: row.get(3)?,
        duration_minutes: row.get(4)?,
        group_id: row.get(5)?,
    })
}

impl SqliteStorage {
    pub(crate) fn get_event_by_id(&self, conn: &Connection, id: i64) -> Result<Event, DomainError> {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT id, title, start_date, start_time, duration_minutes, group_id
             FROM events WHERE id=?1",
            params![id],
            map_event,
        )
        .optional()
        .map_err(map_err)?
        .ok_or_else(|| DomainError::NotFound(format!("Event {id} not found")))
    }
}

impl EventRepository for SqliteStorage {
    fn list_events(
        &self,
        from_date: Option<&str>,
        to_date: Option<&str>,
    ) -> Result<Vec<Event>, DomainError> {
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
            "SELECT id, title, start_date, start_time, duration_minutes, group_id
             FROM events
             {where_clause}
             ORDER BY start_date ASC, start_time ASC"
        );

        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter()),
                map_event,
            )
            .map_err(map_err)?;

        let mut events = vec![];
        for row in rows {
            events.push(row.map_err(map_err)?);
        }
        Ok(events)
    }

    fn create_event(&self, input: NewEvent) -> Result<Event, DomainError> {
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
                return Err(DomainError::ForeignKeyViolation(format!(
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
        .map_err(map_err)?;

        let id = conn.last_insert_rowid();
        self.get_event_by_id(&conn, id)
    }

    fn update_event(&self, id: i64, patch: EventPatch) -> Result<Event, DomainError> {
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
                return Err(DomainError::ForeignKeyViolation(format!(
                    "group_id {gid} does not exist"
                )));
            }
        }

        conn.execute(
            "UPDATE events SET title=?1, start_date=?2, start_time=?3,
             duration_minutes=?4, group_id=?5 WHERE id=?6",
            params![title, start_date, start_time, duration_minutes, group_id, id],
        )
        .map_err(map_err)?;

        self.get_event_by_id(&conn, id)
    }

    fn delete_event(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        self.get_event_by_id(&conn, id)?;
        conn.execute("DELETE FROM events WHERE id=?1", params![id])
            .map_err(map_err)?;
        Ok(())
    }
}
