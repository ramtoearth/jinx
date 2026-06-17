use domain::{
    DomainError, Group, GroupInfo, GroupsSnapshot, HexColor,
    group::{GroupRepository, NewGroup},
};
use rusqlite::{Connection, OptionalExtension, params};

use super::{SqliteStorage, map_err};

fn map_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<Group> {
    let color_str: String = row.get(2)?;
    Ok(Group {
        id: row.get(0)?,
        name: row.get(1)?,
        color: HexColor::new(color_str).unwrap_or_else(|_| HexColor::new("#808080").unwrap()),
    })
}

impl SqliteStorage {
    pub(crate) fn get_group_by_id(&self, conn: &Connection, id: i64) -> Result<Group, DomainError> {
        conn.query_row(
            "SELECT id, name, color FROM groups WHERE id=?1",
            params![id],
            map_group,
        )
        .optional()
        .map_err(map_err)?
        .ok_or_else(|| DomainError::NotFound(format!("Group {id} not found")))
    }
}

impl GroupRepository for SqliteStorage {
    fn list_groups(&self) -> Result<Vec<Group>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, name, color FROM groups ORDER BY id ASC")
            .map_err(map_err)?;
        let rows = stmt.query_map([], map_group).map_err(map_err)?;
        let mut groups = vec![];
        for row in rows {
            groups.push(row.map_err(map_err)?);
        }
        Ok(groups)
    }

    fn find_group_by_name(&self, name: &str) -> Result<Option<Group>, DomainError> {
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
                            &row.get::<_, String>(2).unwrap_or_default()
                        ).unwrap_or_else(|_| HexColor::new("#000000").unwrap()),
                    })
                },
            )
            .optional()
            .map_err(map_err)?;
        Ok(result)
    }

    fn create_group(&self, input: NewGroup) -> Result<Group, DomainError> {
        let conn = self.conn.lock().unwrap();
        match conn.execute(
            "INSERT INTO groups (name, color) VALUES (?1, ?2)",
            params![input.name, input.color.as_str()],
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(ref e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(DomainError::GroupNameNotUnique(format!(
                    "Ya existe un Grupo llamado '{}'",
                    input.name
                )));
            }
            Err(e) => return Err(map_err(e)),
        }
        let id = conn.last_insert_rowid();
        self.get_group_by_id(&conn, id)
    }

    fn rename_group(&self, id: i64, name: String) -> Result<Group, DomainError> {
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
                return Err(DomainError::GroupNameNotUnique(format!(
                    "Ya existe un Grupo llamado '{name}'"
                )));
            }
            Err(e) => return Err(map_err(e)),
        }
        self.get_group_by_id(&conn, id)
    }

    fn recolor_group(&self, id: i64, color: HexColor) -> Result<Group, DomainError> {
        let conn = self.conn.lock().unwrap();
        self.get_group_by_id(&conn, id)?;
        conn.execute(
            "UPDATE groups SET color=?1 WHERE id=?2",
            params![color.as_str(), id],
        )
        .map_err(map_err)?;
        self.get_group_by_id(&conn, id)
    }

    fn delete_group(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        self.get_group_by_id(&conn, id)?;
        conn.execute("DELETE FROM groups WHERE id=?1", params![id])
            .map_err(map_err)?;
        Ok(())
    }

    fn snapshot_for_inference(&self) -> Result<GroupsSnapshot, DomainError> {
        let conn = self.conn.lock().unwrap();
        let groups = {
            let mut stmt = conn
                .prepare("SELECT id, name, color FROM groups ORDER BY id ASC")
                .map_err(map_err)?;
            let rows = stmt.query_map([], map_group).map_err(map_err)?;
            let mut gs = vec![];
            for row in rows {
                gs.push(row.map_err(map_err)?);
            }
            gs
        };

        let mut snapshot = vec![];
        for group in &groups {
            let mut titles: Vec<String> = vec![];
            {
                let mut stmt = conn
                    .prepare("SELECT title FROM tasks WHERE group_id=?1")
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(params![group.id], |r| r.get(0))
                    .map_err(map_err)?;
                for row in rows {
                    titles.push(row.map_err(map_err)?);
                }
            }
            {
                let mut stmt = conn
                    .prepare("SELECT title FROM events WHERE group_id=?1")
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(params![group.id], |r| r.get(0))
                    .map_err(map_err)?;
                for row in rows {
                    titles.push(row.map_err(map_err)?);
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
}
