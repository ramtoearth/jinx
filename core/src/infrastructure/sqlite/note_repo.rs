use std::path::{Path, PathBuf};

use crate::domain::{
    DomainError, Note,
    note::{NewNote, NoteRepository, NotePatch},
};
use rusqlite::{Connection, params};

use super::{SqliteStorage, map_err, now_iso};

fn map_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

impl SqliteStorage {
    pub(crate) fn get_note_by_id(&self, conn: &Connection, id: i64) -> Result<Note, DomainError> {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT id, title, body, created_at, updated_at FROM notes WHERE id=?1",
            params![id],
            map_note,
        )
        .optional()
        .map_err(map_err)?
        .ok_or_else(|| DomainError::NotFound(format!("Note {id} not found")))
    }
}

impl NoteRepository for SqliteStorage {
    fn list_notes(&self) -> Result<Vec<Note>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, title, body, created_at, updated_at FROM notes ORDER BY updated_at DESC")
            .map_err(map_err)?;
        let rows = stmt.query_map([], map_note).map_err(map_err)?;
        let mut notes = vec![];
        for row in rows {
            notes.push(row.map_err(map_err)?);
        }
        Ok(notes)
    }

    fn search_notes(&self, query: &str) -> Result<Vec<Note>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let words: Vec<&str> = query.split_whitespace().filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            return Ok(vec![]);
        }

        let conditions: Vec<String> = words
            .iter()
            .map(|_| "(title LIKE ? OR body LIKE ?)".to_string())
            .collect();
        let where_clause = conditions.join(" OR ");
        let sql = format!(
            "SELECT id, title, body, created_at, updated_at FROM notes
             WHERE {where_clause}
             ORDER BY updated_at DESC"
        );

        let patterns: Vec<String> = words
            .iter()
            .flat_map(|w| {
                let char_count = w.chars().count();
                let prefix = if char_count > 4 {
                    let end = w.char_indices().nth(char_count - 2).map(|(i, _)| i).unwrap_or(w.len());
                    &w[..end]
                } else {
                    w
                };
                let p = format!("%{prefix}%");
                vec![p.clone(), p]
            })
            .collect();
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(patterns.iter()), map_note)
            .map_err(map_err)?;
        let mut notes = vec![];
        for row in rows {
            notes.push(row.map_err(map_err)?);
        }
        Ok(notes)
    }

    fn create_note(&self, input: NewNote) -> Result<Note, DomainError> {
        let conn = self.conn.lock().unwrap();
        let ts = now_iso();
        conn.execute(
            "INSERT INTO notes (title, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![input.title, input.body, ts, ts],
        )
        .map_err(map_err)?;
        let id = conn.last_insert_rowid();
        self.get_note_by_id(&conn, id)
    }

    fn update_note(&self, id: i64, patch: NotePatch) -> Result<Note, DomainError> {
        let conn = self.conn.lock().unwrap();
        let existing = self.get_note_by_id(&conn, id)?;
        let title = patch.title.unwrap_or(existing.title);
        let body = patch.body.unwrap_or(existing.body);
        let updated_at = now_iso();
        conn.execute(
            "UPDATE notes SET title=?1, body=?2, updated_at=?3 WHERE id=?4",
            params![title, body, updated_at, id],
        )
        .map_err(map_err)?;
        self.get_note_by_id(&conn, id)
    }

    fn delete_note(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute("DELETE FROM notes WHERE id=?1", params![id])
            .map_err(map_err)?;
        if affected == 0 {
            return Err(DomainError::NotFound(format!("Note {id} not found")));
        }
        Ok(())
    }

    fn export_note(&self, id: i64, output_path: &Path) -> Result<PathBuf, DomainError> {
        let conn = self.conn.lock().unwrap();
        let note = self.get_note_by_id(&conn, id)?;
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DomainError::IoError(format!("cannot create directory: {e}")))?;
        }
        let content = format!("# {}\n\n{}\n", note.title, note.body);
        std::fs::write(output_path, &content)
            .map_err(|e| DomainError::IoError(format!("cannot write file: {e}")))?;
        Ok(output_path.to_path_buf())
    }
}
