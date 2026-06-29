use crate::db::{self, Database};
use crate::google::tasks_api::{GoogleTask, GoogleTasksClient};
use crate::models::{SyncState, Task, TaskList};
use crate::sync::conflict::{resolve_conflict, ConflictResolution};
use crate::sync::recurrence;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tracing::{info, warn, error};

#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub lists_pulled: usize,
    pub lists_pushed: usize,
    pub tasks_pulled: usize,
    pub tasks_pushed: usize,
    pub tasks_deleted: usize,
    pub conflicts_resolved: Vec<String>,
}

pub async fn run_sync(db: &Database, client: &mut GoogleTasksClient) -> Result<SyncReport, String> {
    info!("Starting TaskFlow bidirectional sync...");
    let mut report = SyncReport::default();

    // ==========================================
    // 1. SYNC TASK LISTS
    // ==========================================
    let remote_lists = client.list_task_lists().await
        .map_err(|e| format!("Failed to list remote task lists: {}", e))?;

    let mut remote_lists_map: HashMap<String, _> = remote_lists.into_iter()
        .map(|l| (l.id.clone(), l))
        .collect();

    // Update or insert remote lists locally
    {
        let conn = db.connect().map_err(|e| e.to_string())?;
        for (remote_id, remote_list) in &remote_lists_map {
            let existing = db::task_lists::get_by_google_id(&conn, remote_id)
                .map_err(|e| format!("DB error: {}", e))?;

            if let Some(mut local_list) = existing {
                if local_list.title != remote_list.title {
                    local_list.title = remote_list.title.clone();
                    local_list.updated_at = Utc::now();
                    db::task_lists::update(&conn, &local_list)
                        .map_err(|e| format!("Failed to update local list: {}", e))?;
                }
            } else {
                let new_list = TaskList {
                    id: uuid::Uuid::new_v4().to_string(),
                    google_id: Some(remote_id.clone()),
                    title: remote_list.title.clone(),
                    position: 0,
                    is_default: false,
                    updated_at: Utc::now(),
                };
                db::task_lists::create(&conn, &new_list)
                    .map_err(|e| format!("Failed to create local list: {}", e))?;
                report.lists_pulled += 1;
            }
        }
    }

    // Handle local lists not yet pushed
    let mut pending_lists = {
        let conn = db.connect().map_err(|e| e.to_string())?;
        db::task_lists::get_all(&conn).map_err(|e| e.to_string())?
    };
    
    for local_list in &mut pending_lists {
        if local_list.google_id.is_none() {
            info!("Pushing new local list: {}", local_list.title);
            match client.create_task_list(&local_list.title).await {
                Ok(remote_list) => {
                    let remote_id = remote_list.id.clone();
                    remote_lists_map.insert(remote_id.clone(), remote_list);
                    local_list.google_id = Some(remote_id);
                    local_list.updated_at = Utc::now();
                    
                    let conn = db.connect().map_err(|e| e.to_string())?;
                    db::task_lists::update(&conn, local_list)
                        .map_err(|e| format!("Failed to update list google_id: {}", e))?;
                    report.lists_pushed += 1;
                }
                Err(e) => {
                    error!("Failed to push local list {}: {}", local_list.title, e);
                }
            }
        }
    }

    // Clean up local lists that were deleted remotely
    {
        let conn = db.connect().map_err(|e| e.to_string())?;
        let current_local_lists = db::task_lists::get_all(&conn)
            .map_err(|e| format!("DB error: {}", e))?;
        for local_list in &current_local_lists {
            if let Some(ref gid) = local_list.google_id {
                if !remote_lists_map.contains_key(gid) {
                    warn!("List '{}' deleted remotely. Deleting locally.", local_list.title);
                    db::task_lists::delete(&conn, &local_list.id)
                        .map_err(|e| format!("Failed to delete local list: {}", e))?;
                }
            }
        }
    }

    // ==========================================
    // 2. SYNC TASKS (PER LIST)
    // ==========================================
    // Reload lists to sync their tasks
    let active_lists = {
        let conn = db.connect().map_err(|e| e.to_string())?;
        db::task_lists::get_all(&conn).map_err(|e| format!("DB error: {}", e))?
    };

    // Load last sync time
    let last_sync_time = {
        let conn = db.connect().map_err(|e| e.to_string())?;
        let last_sync_str = db::sync_meta::get(&conn, "last_full_sync_at")
            .map_err(|e| format!("DB error: {}", e))?;
        last_sync_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc))
    };

    // Get 5-second overlap window
    let updated_min = last_sync_time.map(|t| t - chrono::Duration::seconds(5));

    for list in active_lists {
        let google_list_id = match &list.google_id {
            Some(gid) => gid,
            None => continue,
        };

        // PULL REMOTE CHANGES
        let remote_tasks = match client.list_tasks(google_list_id, updated_min, true, true).await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to fetch remote tasks for list {}: {}", list.title, e);
                continue;
            }
        };

        for remote_task in remote_tasks {
            let google_id = match &remote_task.id {
                Some(id) => id,
                None => continue,
            };

            let existing_task = {
                let conn = db.connect().map_err(|e| e.to_string())?;
                db::tasks::get_by_google_id(&conn, google_id)
                    .map_err(|e| format!("DB error: {}", e))?
            };

            let remote_updated = remote_task.updated.as_ref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            match existing_task {
                None => {
                    if remote_task.deleted == Some(true) {
                        continue;
                    }

                    let local_id = uuid::Uuid::new_v4().to_string();
                    let due_date = remote_task.due.as_ref()
                        .and_then(|s| chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").ok());
                    let completed_at = remote_task.completed.as_ref()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc));

                    let (notes, reminder_time) = extract_reminder_from_notes(&remote_task.notes);

                    let new_task = Task {
                        id: local_id,
                        google_id: Some(google_id.clone()),
                        list_id: list.id.clone(),
                        title: remote_task.title.clone().unwrap_or_default(),
                        notes,
                        status: remote_task.status.clone().unwrap_or_else(|| "needsAction".to_string()),
                        due_date,
                        reminder_time,
                        parent_id: None,
                        position: remote_task.position.clone(),
                        completed_at,
                        updated_at: remote_updated,
                        google_updated_at: Some(remote_updated),
                        sync_state: SyncState::Synced,
                        is_deleted: false,
                        recurrence_rule: None,
                        starred: remote_task.starred.unwrap_or(false),
                    };

                    let conn = db.connect().map_err(|e| e.to_string())?;
                    db::tasks::create(&conn, &new_task)
                        .map_err(|e| format!("Failed to create pulled task: {}", e))?;
                    report.tasks_pulled += 1;
                }
                Some(mut local_task) => {
                    if remote_task.deleted == Some(true) {
                        let conn = db.connect().map_err(|e| e.to_string())?;
                        db::tasks::hard_delete(&conn, &local_task.id)
                            .map_err(|e| format!("Failed to delete local task: {}", e))?;
                        report.tasks_deleted += 1;
                        continue;
                    }

                    match local_task.sync_state {
                        SyncState::Synced => {
                            let due_date = remote_task.due.as_ref()
                                .and_then(|s| chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").ok());
                            let completed_at = remote_task.completed.as_ref()
                                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                                .map(|dt| dt.with_timezone(&Utc));

                            let (notes, reminder_time) = extract_reminder_from_notes(&remote_task.notes);

                            let old_status = local_task.status.clone();
                            local_task.title = remote_task.title.clone().unwrap_or_default();
                            local_task.notes = notes;
                            local_task.status = remote_task.status.clone().unwrap_or_else(|| "needsAction".to_string());
                            local_task.due_date = due_date;
                            local_task.reminder_time = reminder_time;
                            local_task.completed_at = completed_at;
                            local_task.position = remote_task.position.clone();
                            local_task.updated_at = remote_updated;
                            local_task.google_updated_at = Some(remote_updated);
                            local_task.starred = remote_task.starred.unwrap_or(false);
                            local_task.sync_state = SyncState::Synced;

                            let conn = db.connect().map_err(|e| e.to_string())?;
                            db::tasks::update(&conn, &local_task)
                                .map_err(|e| format!("Failed to update local task: {}", e))?;
                            report.tasks_pulled += 1;

                            if old_status != "completed" && local_task.status == "completed" {
                                if let Err(e) = recurrence::handle_recurring_task_completion(&conn, &local_task) {
                                    error!("Failed to handle recurring task spawning: {}", e);
                                }
                            }
                        }
                        SyncState::Pending | SyncState::Conflict => {
                            match resolve_conflict(local_task.updated_at, remote_updated) {
                                ConflictResolution::UseRemote => {
                                    let due_date = remote_task.due.as_ref()
                                        .and_then(|s| chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").ok());
                                    let completed_at = remote_task.completed.as_ref()
                                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                                        .map(|dt| dt.with_timezone(&Utc));

                                    let (notes, reminder_time) = extract_reminder_from_notes(&remote_task.notes);

                                    let old_status = local_task.status.clone();
                                    local_task.title = remote_task.title.clone().unwrap_or_default();
                                    local_task.notes = notes;
                                    local_task.status = remote_task.status.clone().unwrap_or_else(|| "needsAction".to_string());
                                    local_task.due_date = due_date;
                                    local_task.reminder_time = reminder_time;
                                    local_task.completed_at = completed_at;
                                    local_task.position = remote_task.position.clone();
                                    local_task.updated_at = remote_updated;
                                    local_task.google_updated_at = Some(remote_updated);
                                    local_task.starred = remote_task.starred.unwrap_or(false);
                                    local_task.sync_state = SyncState::Synced;

                                    let conn = db.connect().map_err(|e| e.to_string())?;
                                    db::tasks::update(&conn, &local_task)
                                        .map_err(|e| format!("Failed to update local task (conflict): {}", e))?;
                                    
                                    report.conflicts_resolved.push(format!(
                                        "Updated '{}' with remote edits (remote changes were newer)",
                                        local_task.title
                                    ));
                                    report.tasks_pulled += 1;

                                    if old_status != "completed" && local_task.status == "completed" {
                                        if let Err(e) = recurrence::handle_recurring_task_completion(&conn, &local_task) {
                                            error!("Failed to handle recurring task spawning: {}", e);
                                        }
                                    }
                                }
                                ConflictResolution::UseLocal => {
                                    local_task.sync_state = SyncState::Pending;
                                    let conn = db.connect().map_err(|e| e.to_string())?;
                                    db::tasks::update(&conn, &local_task)
                                        .map_err(|e| format!("Failed to set conflict state: {}", e))?;
                                    
                                    report.conflicts_resolved.push(format!(
                                        "Kept your local edits for '{}' (local changes were newer)",
                                        local_task.title
                                    ));
                                }
                            }
                        }
                        SyncState::DeletedPending => {
                            if remote_updated > local_task.updated_at {
                                let due_date = remote_task.due.as_ref()
                                    .and_then(|s| chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").ok());
                                let completed_at = remote_task.completed.as_ref()
                                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                                    .map(|dt| dt.with_timezone(&Utc));

                                let (notes, reminder_time) = extract_reminder_from_notes(&remote_task.notes);

                                local_task.title = remote_task.title.clone().unwrap_or_default();
                                local_task.notes = notes;
                                local_task.status = remote_task.status.clone().unwrap_or_else(|| "needsAction".to_string());
                                local_task.due_date = due_date;
                                local_task.reminder_time = reminder_time;
                                local_task.completed_at = completed_at;
                                local_task.position = remote_task.position.clone();
                                local_task.is_deleted = false;
                                local_task.updated_at = remote_updated;
                                local_task.google_updated_at = Some(remote_updated);
                                local_task.sync_state = SyncState::Synced;

                                let conn = db.connect().map_err(|e| e.to_string())?;
                                db::tasks::update(&conn, &local_task)
                                    .map_err(|e| format!("Failed to restore task: {}", e))?;

                                report.conflicts_resolved.push(format!(
                                    "Restored deleted task '{}' because it was modified remotely",
                                    local_task.title
                                ));
                                report.tasks_pulled += 1;
                            }
                        }
                    }
                }
            }
        }

        // RESOLVE PARENT MAPPINGS
        let remote_tasks_verify = match client.list_tasks(google_list_id, None, true, true).await {
            Ok(t) => t,
            Err(_) => Vec::new(),
        };

        {
            let conn = db.connect().map_err(|e| e.to_string())?;
            for r_task in remote_tasks_verify {
                if let (Some(gid), Some(parent_gid)) = (&r_task.id, &r_task.parent) {
                    if let Some(mut local_task) = db::tasks::get_by_google_id(&conn, gid).unwrap_or(None) {
                        if let Some(parent_task) = db::tasks::get_by_google_id(&conn, parent_gid).unwrap_or(None) {
                            if local_task.parent_id.as_ref() != Some(&parent_task.id) {
                                local_task.parent_id = Some(parent_task.id);
                                let _ = db::tasks::update(&conn, &local_task);
                            }
                        }
                    }
                }
            }
        }

        // PUSH DELETIONS
        let deleted_pending_tasks = {
            let conn = db.connect().map_err(|e| e.to_string())?;
            db::tasks::get_deleted_pending(&conn).map_err(|e| e.to_string())?
        };

        for del_task in deleted_pending_tasks {
            if del_task.list_id == list.id {
                if let Some(ref gid) = del_task.google_id {
                    info!("Pushing task deletion to Google: {}", del_task.title);
                    match client.delete_task(google_list_id, gid).await {
                        Ok(_) => {}
                        Err(e) => {
                            if !e.contains("404") {
                                warn!("Failed to delete task remotely: {}", e);
                            }
                        }
                    }
                }
                let conn = db.connect().map_err(|e| e.to_string())?;
                db::tasks::hard_delete(&conn, &del_task.id)
                    .map_err(|e| format!("Failed to hard delete task: {}", e))?;
                report.tasks_deleted += 1;
            }
        }

        // PUSH ADDITIONS/MODIFICATIONS (Two-pass)
        let pending_tasks = {
            let conn = db.connect().map_err(|e| e.to_string())?;
            db::tasks::get_pending(&conn).map_err(|e| e.to_string())?
        };

        let mut pass1 = Vec::new();
        let mut pass2 = Vec::new();

        for t in pending_tasks {
            if t.list_id == list.id {
                if t.parent_id.is_none() {
                    pass1.push(t);
                } else {
                    pass2.push(t);
                }
            }
        }

        for task in pass1 {
            push_task_to_google(db, client, google_list_id, task, &mut report).await?;
        }

        for task in pass2 {
            push_task_to_google(db, client, google_list_id, task, &mut report).await?;
        }
    }

    // Save sync completion timestamp
    {
        let conn = db.connect().map_err(|e| e.to_string())?;
        db::sync_meta::set(&conn, "last_full_sync_at", &Utc::now().to_rfc3339())
            .map_err(|e| format!("Failed to save last sync timestamp: {}", e))?;
    }

    info!("TaskFlow sync finished successfully.");
    Ok(report)
}

async fn push_task_to_google(
    db: &Database,
    client: &mut GoogleTasksClient,
    google_list_id: &str,
    mut task: Task,
    report: &mut SyncReport,
) -> Result<(), String> {
    let parent_google_id = if let Some(ref pid) = task.parent_id {
        let conn = db.connect().map_err(|e| e.to_string())?;
        let parent_task = db::tasks::get(&conn, pid)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "Parent task not found in database".to_string())?;
        parent_task.google_id
    } else {
        None
    };

    let mut notes_with_reminder = task.notes.clone();
    if let Some(reminder) = task.reminder_time {
        let reminder_str = format!("[Reminder: {}]", reminder.format("%H:%M:%S"));
        if let Some(ref existing_notes) = task.notes {
            let cleaned_existing = clean_reminder_from_notes(existing_notes);
            if cleaned_existing.is_empty() {
                notes_with_reminder = Some(reminder_str);
            } else {
                notes_with_reminder = Some(format!("{}\n{}", cleaned_existing, reminder_str));
            }
        } else {
            notes_with_reminder = Some(reminder_str);
        }
    } else if let Some(ref existing_notes) = task.notes {
        let cleaned = clean_reminder_from_notes(existing_notes);
        notes_with_reminder = if cleaned.is_empty() { None } else { Some(cleaned) };
    }

    let google_payload = GoogleTask {
        id: task.google_id.clone(),
        title: Some(task.title.clone()),
        notes: notes_with_reminder,
        status: Some(task.status.clone()),
        due: task.due_date.map(|d| format!("{}T00:00:00.000Z", d.format("%Y-%m-%d"))),
        completed: task.completed_at.map(|c| c.to_rfc3339()),
        updated: None,
        parent: None,
        position: None,
        deleted: None,
        hidden: None,
        starred: Some(task.starred),
    };

    if task.google_id.is_none() {
        // INSERT
        info!("Creating task on Google: {}", task.title);
        match client.insert_task(google_list_id, &google_payload, parent_google_id.as_deref(), None).await {
            Ok(created) => {
                let created_updated = created.updated.as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                task.google_id = created.id;
                task.google_updated_at = Some(created_updated);
                task.position = created.position;
                task.sync_state = SyncState::Synced;

                let conn = db.connect().map_err(|e| e.to_string())?;
                db::tasks::update(&conn, &task)
                    .map_err(|e| format!("Failed to update synced task: {}", e))?;
                report.tasks_pushed += 1;
            }
            Err(e) => {
                error!("Failed to create task {} on Google: {}", task.title, e);
            }
        }
    } else {
        // UPDATE
        let gid = task.google_id.as_ref().unwrap();
        info!("Updating task on Google: {}", task.title);
        match client.update_task(google_list_id, gid, &google_payload).await {
            Ok(updated) => {
                let updated_time = updated.updated.as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                if parent_google_id.is_some() {
                    let _ = client.move_task(google_list_id, gid, parent_google_id.as_deref(), None).await;
                }

                task.google_updated_at = Some(updated_time);
                task.sync_state = SyncState::Synced;

                let conn = db.connect().map_err(|e| e.to_string())?;
                db::tasks::update(&conn, &task)
                    .map_err(|e| format!("Failed to update synced task: {}", e))?;
                report.tasks_pushed += 1;
            }
            Err(e) => {
                error!("Failed to update task {} on Google: {}", task.title, e);
            }
        }
    }
    Ok(())
}

fn clean_reminder_from_notes(notes: &str) -> String {
    let mut lines = Vec::new();
    for line in notes.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[Reminder:") && trimmed.ends_with(']') {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

fn extract_reminder_from_notes(notes: &Option<String>) -> (Option<String>, Option<chrono::NaiveTime>) {
    let notes_str = match notes {
        Some(n) => n,
        None => return (None, None),
    };

    let mut cleaned_lines = Vec::new();
    let mut reminder_time = None;

    for line in notes_str.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[Reminder:") && trimmed.ends_with(']') {
            let time_part = &trimmed[10..trimmed.len() - 1];
            if let Ok(time) = chrono::NaiveTime::parse_from_str(time_part, "%H:%M:%S") {
                reminder_time = Some(time);
            } else if let Ok(time) = chrono::NaiveTime::parse_from_str(time_part, "%H:%M") {
                reminder_time = Some(time);
            }
            continue;
        }
        cleaned_lines.push(line);
    }

    let cleaned_notes = cleaned_lines.join("\n").trim().to_string();
    let final_notes = if cleaned_notes.is_empty() {
        None
    } else {
        Some(cleaned_notes)
    };

    (final_notes, reminder_time)
}
