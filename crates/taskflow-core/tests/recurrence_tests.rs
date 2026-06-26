use taskflow_core::db::{self, Database};
use taskflow_core::models::{RecurrenceRule, SyncState, Task, TaskList};
use taskflow_core::sync::recurrence::{compute_next_due_date, handle_recurring_task_completion};
use chrono::{NaiveDate, NaiveTime, Utc, Weekday};
use uuid::Uuid;

#[test]
fn test_recurrence_date_calculation() {
    let start = NaiveDate::from_ymd_opt(2026, 6, 24).unwrap(); // Wednesday

    // Daily
    let next_daily = compute_next_due_date(start, &RecurrenceRule::Daily);
    assert_eq!(next_daily, NaiveDate::from_ymd_opt(2026, 6, 25).unwrap()); // Thursday

    // Weekly (same day next week)
    let next_weekly_same = compute_next_due_date(start, &RecurrenceRule::Weekly(Weekday::Wed));
    assert_eq!(next_weekly_same, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()); // Next Wed

    // Weekly (different day)
    let next_weekly_diff = compute_next_due_date(start, &RecurrenceRule::Weekly(Weekday::Fri));
    assert_eq!(next_weekly_diff, NaiveDate::from_ymd_opt(2026, 6, 26).unwrap()); // Next Fri

    // Monthly
    let next_monthly = compute_next_due_date(start, &RecurrenceRule::Monthly(24));
    assert_eq!(next_monthly, NaiveDate::from_ymd_opt(2026, 7, 24).unwrap()); // Next Month 24th

    // Monthly day-capping (Jan 31st -> Feb 28th)
    let jan_31 = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    let next_monthly_cap = compute_next_due_date(jan_31, &RecurrenceRule::Monthly(31));
    assert_eq!(next_monthly_cap, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()); // End of Feb
}

#[test]
fn test_handle_recurring_task_completion() {
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

    let task_id = Uuid::new_v4().to_string();
    let task = Task {
        id: task_id.clone(),
        google_id: Some("g_recurring_1".to_string()),
        list_id: list_id.clone(),
        title: "Submit Timesheet".to_string(),
        notes: Some("Weekly task".to_string()),
        status: "completed".to_string(), // Marked completed
        due_date: Some(NaiveDate::from_ymd_opt(2026, 6, 24).unwrap()), // Wed
        reminder_time: Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
        parent_id: None,
        position: Some("position_a".to_string()),
        completed_at: Some(Utc::now()),
        updated_at: Utc::now(),
        google_updated_at: None,
        sync_state: SyncState::Synced,
        is_deleted: false,
        recurrence_rule: Some(RecurrenceRule::Weekly(Weekday::Wed)),
        starred: false,
    };
    db::tasks::create(&conn, &task).unwrap();

    // Spawn next occurrence
    let result = handle_recurring_task_completion(&conn, &task).unwrap();
    assert!(result.is_some());
    let next_occurrence = result.unwrap();

    // Verify properties of spawned occurrence
    assert_eq!(next_occurrence.title, "Submit Timesheet");
    assert_eq!(next_occurrence.notes, Some("Weekly task".to_string()));
    assert_eq!(next_occurrence.status, "needsAction"); // Reset
    assert_eq!(next_occurrence.due_date, Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())); // Next Wed
    assert_eq!(next_occurrence.reminder_time, Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap()));
    assert_eq!(next_occurrence.google_id, None); // Null, triggers insert push
    assert_eq!(next_occurrence.sync_state, SyncState::Pending);

    // Verify it exists in local database
    let fetched = db::tasks::get(&conn, &next_occurrence.id).unwrap().unwrap();
    assert_eq!(fetched.title, "Submit Timesheet");
    assert_eq!(fetched.due_date, Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()));
}
