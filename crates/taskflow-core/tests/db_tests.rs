use taskflow_core::db::{self, Database};
use taskflow_core::models::{RecurrenceRule, SyncState, Task, TaskList};
use chrono::{Utc, NaiveDate, NaiveTime, Weekday};
use uuid::Uuid;

#[test]
fn test_database_in_memory_initialization() {
    let db = Database::in_memory();
    let conn = db.connect().unwrap();
    
    // Schema should be initialized and writable
    db::sync_meta::set(&conn, "test_key", "test_value").unwrap();
    let val = db::sync_meta::get(&conn, "test_key").unwrap();
    assert_eq!(val, Some("test_value".to_string()));
}

#[test]
fn test_task_list_crud() {
    let db = Database::in_memory();
    let conn = db.connect().unwrap();

    let list_id = Uuid::new_v4().to_string();
    let list = TaskList {
        id: list_id.clone(),
        google_id: Some("g_list_123".to_string()),
        title: "Test List".to_string(),
        position: 0,
        is_default: true,
        updated_at: Utc::now(),
    };

    // Create
    db::task_lists::create(&conn, &list).unwrap();

    // Get
    let fetched = db::task_lists::get(&conn, &list_id).unwrap().unwrap();
    assert_eq!(fetched.title, "Test List");
    assert_eq!(fetched.google_id, Some("g_list_123".to_string()));
    assert!(fetched.is_default);

    // Get by google_id
    let fetched_g = db::task_lists::get_by_google_id(&conn, "g_list_123").unwrap().unwrap();
    assert_eq!(fetched_g.id, list_id);

    // Get all
    let all = db::task_lists::get_all(&conn).unwrap();
    assert_eq!(all.len(), 1);

    // Update
    let mut updated_list = fetched;
    updated_list.title = "Updated List Title".to_string();
    updated_list.is_default = false;
    db::task_lists::update(&conn, &updated_list).unwrap();

    let fetched_updated = db::task_lists::get(&conn, &list_id).unwrap().unwrap();
    assert_eq!(fetched_updated.title, "Updated List Title");
    assert!(!fetched_updated.is_default);

    // Delete
    db::task_lists::delete(&conn, &list_id).unwrap();
    let deleted = db::task_lists::get(&conn, &list_id).unwrap();
    assert!(deleted.is_none());
}

#[test]
fn test_task_crud() {
    let db = Database::in_memory();
    let conn = db.connect().unwrap();

    // Setup list first for reference constraint
    let list_id = Uuid::new_v4().to_string();
    let list = TaskList {
        id: list_id.clone(),
        google_id: Some("g_list_id".to_string()),
        title: "Main List".to_string(),
        position: 0,
        is_default: true,
        updated_at: Utc::now(),
    };
    db::task_lists::create(&conn, &list).unwrap();

    let task_id = Uuid::new_v4().to_string();
    let task = Task {
        id: task_id.clone(),
        google_id: None,
        list_id: list_id.clone(),
        title: "Buy Groceries".to_string(),
        notes: Some("Milk and eggs".to_string()),
        status: "needsAction".to_string(),
        due_date: Some(NaiveDate::from_ymd_opt(2026, 6, 24).unwrap()),
        reminder_time: Some(NaiveTime::from_hms_opt(18, 30, 0).unwrap()),
        parent_id: None,
        position: Some("a".to_string()),
        completed_at: None,
        updated_at: Utc::now(),
        google_updated_at: None,
        sync_state: SyncState::Pending,
        is_deleted: false,
        recurrence_rule: Some(RecurrenceRule::Weekly(Weekday::Wed)),
    };

    // Create
    db::tasks::create(&conn, &task).unwrap();

    // Get
    let fetched = db::tasks::get(&conn, &task_id).unwrap().unwrap();
    assert_eq!(fetched.title, "Buy Groceries");
    assert_eq!(fetched.notes, Some("Milk and eggs".to_string()));
    assert_eq!(fetched.due_date, Some(NaiveDate::from_ymd_opt(2026, 6, 24).unwrap()));
    assert_eq!(fetched.reminder_time, Some(NaiveTime::from_hms_opt(18, 30, 0).unwrap()));
    assert_eq!(fetched.sync_state, SyncState::Pending);
    assert_eq!(fetched.recurrence_rule, Some(RecurrenceRule::Weekly(Weekday::Wed)));

    // Update
    let mut updated_task = fetched;
    updated_task.status = "completed".to_string();
    updated_task.completed_at = Some(Utc::now());
    updated_task.google_id = Some("g_task_999".to_string());
    updated_task.sync_state = SyncState::Synced;
    db::tasks::update(&conn, &updated_task).unwrap();

    let fetched_updated = db::tasks::get(&conn, &task_id).unwrap().unwrap();
    assert_eq!(fetched_updated.status, "completed");
    assert_eq!(fetched_updated.google_id, Some("g_task_999".to_string()));
    assert_eq!(fetched_updated.sync_state, SyncState::Synced);
    assert!(fetched_updated.completed_at.is_some());

    // Get all active
    let active_tasks = db::tasks::get_all_active_in_list(&conn, &list_id).unwrap();
    assert_eq!(active_tasks.len(), 1);

    // Soft delete
    db::tasks::soft_delete(&conn, &task_id).unwrap();
    let soft_deleted = db::tasks::get(&conn, &task_id).unwrap().unwrap();
    assert!(soft_deleted.is_deleted);
    assert_eq!(soft_deleted.sync_state, SyncState::DeletedPending);

    // Active query should not return soft deleted tasks
    let active_tasks_post = db::tasks::get_all_active_in_list(&conn, &list_id).unwrap();
    assert_eq!(active_tasks_post.len(), 0);

    // Hard delete
    db::tasks::hard_delete(&conn, &task_id).unwrap();
    let hard_deleted = db::tasks::get(&conn, &task_id).unwrap();
    assert!(hard_deleted.is_none());
}

#[test]
fn test_subtask_and_upcoming_queries() {
    let db = Database::in_memory();
    let conn = db.connect().unwrap();

    let list_id = Uuid::new_v4().to_string();
    let list = TaskList {
        id: list_id.clone(),
        google_id: None,
        title: "Test List".to_string(),
        position: 0,
        is_default: true,
        updated_at: Utc::now(),
    };
    db::task_lists::create(&conn, &list).unwrap();

    let parent_id = Uuid::new_v4().to_string();
    let parent = Task {
        id: parent_id.clone(),
        google_id: None,
        list_id: list_id.clone(),
        title: "Parent Task".to_string(),
        notes: None,
        status: "needsAction".to_string(),
        due_date: Some(Utc::now().naive_utc().date()), // due today
        reminder_time: None,
        parent_id: None,
        position: Some("a".to_string()),
        completed_at: None,
        updated_at: Utc::now(),
        google_updated_at: None,
        sync_state: SyncState::Pending,
        is_deleted: false,
        recurrence_rule: None,
    };
    db::tasks::create(&conn, &parent).unwrap();

    let child_id = Uuid::new_v4().to_string();
    let child = Task {
        id: child_id.clone(),
        google_id: None,
        list_id: list_id.clone(),
        title: "Child Task".to_string(),
        notes: None,
        status: "needsAction".to_string(),
        due_date: Some(Utc::now().naive_utc().date() + chrono::Duration::days(2)), // due in 2 days
        reminder_time: None,
        parent_id: Some(parent_id.clone()),
        position: Some("b".to_string()),
        completed_at: None,
        updated_at: Utc::now(),
        google_updated_at: None,
        sync_state: SyncState::Pending,
        is_deleted: false,
        recurrence_rule: None,
    };
    db::tasks::create(&conn, &child).unwrap();

    // Query subtasks
    let subtasks = db::tasks::get_subtasks(&conn, &parent_id).unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].id, child_id);

    // Query today
    let today_tasks = db::tasks::get_today(&conn).unwrap();
    assert_eq!(today_tasks.len(), 1);
    assert_eq!(today_tasks[0].id, parent_id);

    // Query upcoming next 3 days
    let upcoming_tasks = db::tasks::get_upcoming(&conn, 3).unwrap();
    // child task is due in 2 days, parent task is due today.
    // wait, get_upcoming query specifies `due_date >= today AND due_date <= today + 3`.
    // So both parent and child should match.
    assert_eq!(upcoming_tasks.len(), 2);
}
