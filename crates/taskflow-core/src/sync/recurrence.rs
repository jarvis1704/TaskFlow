use crate::db;
use crate::models::{RecurrenceRule, SyncState, Task};
use chrono::{Datelike, NaiveDate, Utc};
use rusqlite::Connection;
use uuid::Uuid;

/// Calculate the next occurrence date based on the current due date and the recurrence rule
pub fn compute_next_due_date(current_due: NaiveDate, rule: &RecurrenceRule) -> NaiveDate {
    match rule {
        RecurrenceRule::Daily => current_due + chrono::Duration::days(1),
        RecurrenceRule::Weekly(weekday) => {
            let mut next = current_due + chrono::Duration::days(1);
            while next.weekday() != *weekday {
                next = next + chrono::Duration::days(1);
            }
            next
        }
        RecurrenceRule::Monthly(day) => {
            let mut year = current_due.year();
            let mut month = current_due.month() + 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
            let mut target_day = *day;
            loop {
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, target_day) {
                    break date;
                }
                target_day -= 1;
                if target_day == 0 {
                    break current_due + chrono::Duration::days(30); // fallback
                }
            }
        }
        RecurrenceRule::Custom(_) => {
            // For custom, default to next day for simplicity if RRULE parsing is not set up
            current_due + chrono::Duration::days(1)
        }
    }
}

/// Checks if a task is recurring and completed. If so, spawns its next occurrence in the local database.
pub fn handle_recurring_task_completion(conn: &Connection, task: &Task) -> Result<Option<Task>, rusqlite::Error> {
    if task.status != "completed" {
        return Ok(None);
    }

    let rule = match &task.recurrence_rule {
        Some(r) => r,
        None => return Ok(None),
    };

    // Use the task's due date, or default to today if not specified
    let current_due = task.due_date.unwrap_or_else(|| Utc::now().naive_utc().date());
    let next_due = compute_next_due_date(current_due, rule);

    let next_task = Task {
        id: Uuid::new_v4().to_string(),
        google_id: None,
        list_id: task.list_id.clone(),
        title: task.title.clone(),
        notes: task.notes.clone(),
        status: "needsAction".to_string(),
        due_date: Some(next_due),
        reminder_time: task.reminder_time,
        parent_id: task.parent_id.clone(),
        position: None, // Google will assign position on sync
        completed_at: None,
        updated_at: Utc::now(),
        google_updated_at: None,
        sync_state: SyncState::Pending,
        is_deleted: false,
        recurrence_rule: Some(rule.clone()),
    };

    db::tasks::create(conn, &next_task)?;

    Ok(Some(next_task))
}
