use crate::domain::{
    DomainError, Priority, Task, TaskStatus,
    task::{NewTask, TaskFilter, TaskPatch, TaskRepository},
};
use rusqlite::{Connection, params};

use super::{SqliteStorage, map_err, now_iso};

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let priority_str: String = row.get(2)?;
    let status_str: String = row.get(3)?;
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        priority: priority_str.parse().unwrap_or(Priority::Media),
        status: status_str.parse().unwrap_or(TaskStatus::Pendiente),
        created_at: row.get(4)?,
        deadline: row.get(5)?,
        group_id: row.get(6)?,
    })
}

impl SqliteStorage {
    pub(crate) fn get_task_by_id(&self, conn: &Connection, id: i64) -> Result<Task, DomainError> {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT id, title, priority, status, created_at, deadline, group_id
             FROM tasks WHERE id=?1",
            params![id],
            map_task,
        )
        .optional()
        .map_err(map_err)?
        .ok_or_else(|| DomainError::NotFound(format!("Task {id} not found")))
    }
}

impl TaskRepository for SqliteStorage {
    fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>, DomainError> {
        let conn = self.conn.lock().unwrap();
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

        let sql = format!(
            "SELECT id, title, priority, status, created_at, deadline, group_id
             FROM tasks
             {where_clause}
             ORDER BY
               CASE priority WHEN 'alta' THEN 1 WHEN 'media' THEN 2 ELSE 3 END,
               CASE WHEN deadline IS NULL THEN 1 ELSE 0 END,
               deadline ASC"
        );

        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
                map_task,
            )
            .map_err(map_err)?;

        let mut tasks = vec![];
        for row in rows {
            tasks.push(row.map_err(map_err)?);
        }
        Ok(tasks)
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let words: Vec<&str> = query.split_whitespace().filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            return Ok(vec![]);
        }

        let conditions: Vec<String> = words.iter().map(|_| "title LIKE ?".to_string()).collect();
        let where_clause = conditions.join(" OR ");
        let sql = format!(
            "SELECT id, title, priority, status, created_at, deadline, group_id
             FROM tasks
             WHERE {where_clause}
             ORDER BY
               CASE priority WHEN 'alta' THEN 1 WHEN 'media' THEN 2 ELSE 3 END,
               CASE WHEN deadline IS NULL THEN 1 ELSE 0 END,
               deadline ASC"
        );

        let patterns: Vec<String> = words.iter().map(|w| {
            let char_count = w.chars().count();
            let prefix = if char_count > 4 {
                let end = w.char_indices().nth(char_count - 2).map(|(i, _)| i).unwrap_or(w.len());
                &w[..end]
            } else {
                w
            };
            format!("%{prefix}%")
        }).collect();
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(patterns.iter()),
                map_task,
            )
            .map_err(map_err)?;
        let mut tasks = vec![];
        for row in rows {
            tasks.push(row.map_err(map_err)?);
        }
        Ok(tasks)
    }

    fn create_task(&self, input: NewTask) -> Result<Task, DomainError> {
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
                return Err(DomainError::ForeignKeyViolation(format!(
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
        .map_err(map_err)?;

        let id = conn.last_insert_rowid();
        self.get_task_by_id(&conn, id)
    }

    fn update_task(&self, id: i64, patch: TaskPatch) -> Result<Task, DomainError> {
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
                return Err(DomainError::ForeignKeyViolation(format!(
                    "group_id {gid} does not exist"
                )));
            }
        }

        conn.execute(
            "UPDATE tasks SET title=?1, priority=?2, status=?3, deadline=?4, group_id=?5
             WHERE id=?6",
            params![title, priority.as_str(), status.as_str(), deadline, group_id, id],
        )
        .map_err(map_err)?;

        self.get_task_by_id(&conn, id)
    }

    fn complete_task(&self, id: i64) -> Result<Task, DomainError> {
        let conn = self.conn.lock().unwrap();
        self.get_task_by_id(&conn, id)?;
        conn.execute(
            "UPDATE tasks SET status='completada' WHERE id=?1",
            params![id],
        )
        .map_err(map_err)?;
        self.get_task_by_id(&conn, id)
    }

    fn delete_task(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        self.get_task_by_id(&conn, id)?;
        conn.execute("DELETE FROM tasks WHERE id=?1", params![id])
            .map_err(map_err)?;
        Ok(())
    }
}
