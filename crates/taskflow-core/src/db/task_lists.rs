use crate::models::TaskList;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

fn map_row(row: &Row) -> Result<TaskList, rusqlite::Error> {
    let updated_at_str: String = row.get(5)?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;

    Ok(TaskList {
        id: row.get(0)?,
        google_id: row.get(1)?,
        title: row.get(2)?,
        position: row.get(3)?,
        is_default: row.get(4)?,
        updated_at,
    })
}

pub fn create(conn: &Connection, list: &TaskList) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO task_lists (id, google_id, title, position, is_default, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            list.id,
            list.google_id,
            list.title,
            list.position,
            list.is_default,
            list.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<TaskList>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, google_id, title, position, is_default, updated_at FROM task_lists WHERE id = ?1",
        params![id],
        map_row,
    )
    .optional()
}

pub fn get_by_google_id(conn: &Connection, google_id: &str) -> Result<Option<TaskList>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, google_id, title, position, is_default, updated_at FROM task_lists WHERE google_id = ?1",
        params![google_id],
        map_row,
    )
    .optional()
}

pub fn get_all(conn: &Connection) -> Result<Vec<TaskList>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, google_id, title, position, is_default, updated_at FROM task_lists ORDER BY position ASC",
    )?;
    let list_iter = stmt.query_map([], map_row)?;
    let mut lists = Vec::new();
    for list in list_iter {
        lists.push(list?);
    }
    Ok(lists)
}

pub fn update(conn: &Connection, list: &TaskList) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE task_lists
         SET google_id = ?2, title = ?3, position = ?4, is_default = ?5, updated_at = ?6
         WHERE id = ?1",
        params![
            list.id,
            list.google_id,
            list.title,
            list.position,
            list.is_default,
            list.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM task_lists WHERE id = ?1", params![id])?;
    Ok(())
}
