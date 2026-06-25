use crate::models::{SyncState, Task};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::str::FromStr;

const SELECT_FIELDS: &str = "id, google_id, list_id, title, notes, status, due_date, reminder_time, parent_id, position, completed_at, updated_at, google_updated_at, sync_state, is_deleted, recurrence_rule";

fn map_row(row: &Row) -> Result<Task, rusqlite::Error> {
    let due_date: Option<String> = row.get(6)?;
    let parsed_due_date = due_date
        .map(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d"))
        .transpose()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;

    let reminder_time: Option<String> = row.get(7)?;
    let parsed_reminder_time = reminder_time
        .map(|s| NaiveTime::parse_from_str(&s, "%H:%M:%S"))
        .transpose()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;

    let completed_at_str: Option<String> = row.get(10)?;
    let completed_at = completed_at_str
        .map(|s| DateTime::parse_from_rfc3339(&s))
        .transpose()
        .map(|dt_opt| dt_opt.map(|dt| dt.with_timezone(&Utc)))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            10,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;

    let updated_at_str: String = row.get(11)?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            11,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;

    let google_updated_at_str: Option<String> = row.get(12)?;
    let google_updated_at = google_updated_at_str
        .map(|s| DateTime::parse_from_rfc3339(&s))
        .transpose()
        .map(|dt_opt| dt_opt.map(|dt| dt.with_timezone(&Utc)))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            12,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;

    let sync_state_str: String = row.get(13)?;
    let sync_state = SyncState::from_str(&sync_state_str)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            13,
            rusqlite::types::Type::Text,
            e.into(),
        ))?;

    let is_deleted: i32 = row.get(14)?;

    let recurrence_rule_str: Option<String> = row.get(15)?;
    let recurrence_rule = recurrence_rule_str
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            15,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;

    Ok(Task {
        id: row.get(0)?,
        google_id: row.get(1)?,
        list_id: row.get(2)?,
        title: row.get(3)?,
        notes: row.get(4)?,
        status: row.get(5)?,
        due_date: parsed_due_date,
        reminder_time: parsed_reminder_time,
        parent_id: row.get(8)?,
        position: row.get(9)?,
        completed_at,
        updated_at,
        google_updated_at,
        sync_state,
        is_deleted: is_deleted != 0,
        recurrence_rule,
    })
}

pub fn create(conn: &Connection, task: &Task) -> Result<(), rusqlite::Error> {
    let rrule_str = task.recurrence_rule.as_ref()
        .map(|r| serde_json::to_string(r))
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    conn.execute(
        "INSERT INTO tasks (id, google_id, list_id, title, notes, status, due_date, reminder_time, parent_id, position, completed_at, updated_at, google_updated_at, sync_state, is_deleted, recurrence_rule)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            task.id,
            task.google_id,
            task.list_id,
            task.title,
            task.notes,
            task.status,
            task.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
            task.reminder_time.map(|t| t.format("%H:%M:%S").to_string()),
            task.parent_id,
            task.position,
            task.completed_at.map(|d| d.to_rfc3339()),
            task.updated_at.to_rfc3339(),
            task.google_updated_at.map(|d| d.to_rfc3339()),
            task.sync_state.to_string(),
            if task.is_deleted { 1 } else { 0 },
            rrule_str,
        ],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Task>, rusqlite::Error> {
    let sql = format!("SELECT {} FROM tasks WHERE id = ?1", SELECT_FIELDS);
    conn.query_row(&sql, params![id], map_row).optional()
}

pub fn get_by_google_id(conn: &Connection, google_id: &str) -> Result<Option<Task>, rusqlite::Error> {
    let sql = format!("SELECT {} FROM tasks WHERE google_id = ?1", SELECT_FIELDS);
    conn.query_row(&sql, params![google_id], map_row).optional()
}

pub fn get_all_in_list(conn: &Connection, list_id: &str) -> Result<Vec<Task>, rusqlite::Error> {
    let sql = format!("SELECT {} FROM tasks WHERE list_id = ?1 ORDER BY position ASC, title ASC", SELECT_FIELDS);
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map(params![list_id], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn get_all_active_in_list(conn: &Connection, list_id: &str) -> Result<Vec<Task>, rusqlite::Error> {
    let sql = format!(
        "SELECT {} FROM tasks WHERE list_id = ?1 AND is_deleted = 0 ORDER BY position ASC, title ASC",
        SELECT_FIELDS
    );
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map(params![list_id], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn get_pending(conn: &Connection) -> Result<Vec<Task>, rusqlite::Error> {
    let sql = format!("SELECT {} FROM tasks WHERE sync_state = 'pending' AND is_deleted = 0", SELECT_FIELDS);
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map([], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn get_deleted_pending(conn: &Connection) -> Result<Vec<Task>, rusqlite::Error> {
    let sql = format!("SELECT {} FROM tasks WHERE sync_state = 'deleted_pending' OR (is_deleted = 1 AND google_id IS NOT NULL AND sync_state != 'synced')", SELECT_FIELDS);
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map([], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn get_today(conn: &Connection) -> Result<Vec<Task>, rusqlite::Error> {
    let today = Utc::now().naive_utc().date();
    let today_str = today.format("%Y-%m-%d").to_string();
    let sql = format!(
        "SELECT {} FROM tasks WHERE due_date <= ?1 AND status = 'needsAction' AND is_deleted = 0 ORDER BY due_date ASC, title ASC",
        SELECT_FIELDS
    );
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map(params![today_str], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn get_upcoming(conn: &Connection, days: i64) -> Result<Vec<Task>, rusqlite::Error> {
    let today = Utc::now().naive_utc().date();
    let end_date = today + chrono::Duration::days(days);
    let today_str = today.format("%Y-%m-%d").to_string();
    let end_date_str = end_date.format("%Y-%m-%d").to_string();
    let sql = format!(
        "SELECT {} FROM tasks WHERE due_date >= ?1 AND due_date <= ?2 AND status = 'needsAction' AND is_deleted = 0 ORDER BY due_date ASC, title ASC",
        SELECT_FIELDS
    );
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map(params![today_str, end_date_str], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn get_subtasks(conn: &Connection, parent_id: &str) -> Result<Vec<Task>, rusqlite::Error> {
    let sql = format!(
        "SELECT {} FROM tasks WHERE parent_id = ?1 AND is_deleted = 0 ORDER BY position ASC, title ASC",
        SELECT_FIELDS
    );
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map(params![parent_id], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn get_recurring_completed(conn: &Connection) -> Result<Vec<Task>, rusqlite::Error> {
    let sql = format!(
        "SELECT {} FROM tasks WHERE recurrence_rule IS NOT NULL AND status = 'completed' AND is_deleted = 0",
        SELECT_FIELDS
    );
    let mut stmt = conn.prepare(&sql)?;
    let task_iter = stmt.query_map([], map_row)?;
    let mut tasks = Vec::new();
    for task in task_iter {
        tasks.push(task?);
    }
    Ok(tasks)
}

pub fn update(conn: &Connection, task: &Task) -> Result<(), rusqlite::Error> {
    let rrule_str = task.recurrence_rule.as_ref()
        .map(|r| serde_json::to_string(r))
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    conn.execute(
        "UPDATE tasks
         SET google_id = ?2, list_id = ?3, title = ?4, notes = ?5, status = ?6,
             due_date = ?7, reminder_time = ?8, parent_id = ?9, position = ?10,
             completed_at = ?11, updated_at = ?12, google_updated_at = ?13,
             sync_state = ?14, is_deleted = ?15, recurrence_rule = ?16
         WHERE id = ?1",
        params![
            task.id,
            task.google_id,
            task.list_id,
            task.title,
            task.notes,
            task.status,
            task.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
            task.reminder_time.map(|t| t.format("%H:%M:%S").to_string()),
            task.parent_id,
            task.position,
            task.completed_at.map(|d| d.to_rfc3339()),
            task.updated_at.to_rfc3339(),
            task.google_updated_at.map(|d| d.to_rfc3339()),
            task.sync_state.to_string(),
            if task.is_deleted { 1 } else { 0 },
            rrule_str,
        ],
    )?;
    Ok(())
}

pub fn soft_delete(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE tasks
         SET is_deleted = 1, sync_state = 'deleted_pending', updated_at = ?2
         WHERE id = ?1",
        params![id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub fn hard_delete(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
    Ok(())
}
