use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;
use chrono::{Local, NaiveTime};
use tracing::{info, error, warn};
use tracing_subscriber;

use taskflow_core::db::Database;
use taskflow_core::google::oauth::load_credentials;
use taskflow_core::google::token::TokenManager;
use taskflow_core::google::tasks_api::GoogleTasksClient;
use taskflow_core::sync::engine::run_sync;
use notify_rust::Notification;

#[derive(Debug)]
struct ReminderTask {
    id: String,
    title: String,
    reminder_time: NaiveTime,
}

#[tokio::main]
async fn main() {
    // Initialize tracing/logging
    tracing_subscriber::fmt::init();
    info!("Starting TaskFlow background daemon");

    let db = match Database::new() {
        Ok(database) => database,
        Err(e) => {
            error!("Failed to initialize database path: {}", e);
            return;
        }
    };

    // Run the reminder loop and the sync loop concurrently
    let reminder_db = db.clone();
    let reminder_handle = tokio::spawn(async move {
        run_reminder_loop(reminder_db).await;
    });

    let sync_db = db.clone();
    let sync_handle = tokio::spawn(async move {
        run_sync_loop(sync_db).await;
    });

    let _ = tokio::join!(reminder_handle, sync_handle);
}

async fn run_reminder_loop(db: Database) {
    info!("Starting reminder monitoring loop");
    let mut notified_tasks = HashSet::new();
    let mut current_date = Local::now().date_naive();

    loop {
        // Sleep for 15 seconds between reminder checks
        sleep(Duration::from_secs(15)).await;

        let now = Local::now();
        let today = now.date_naive();
        let current_time = now.time();

        // If the date changed, clear notified cache
        if today != current_date {
            info!("Date changed from {} to {}. Clearing notified cache.", current_date, today);
            notified_tasks.clear();
            current_date = today;
        }

        let conn = match db.connect() {
            Ok(c) => c,
            Err(e) => {
                error!("Reminder loop failed to connect to database: {}", e);
                continue;
            }
        };

        let today_str = today.format("%Y-%m-%d").to_string();
        let mut stmt = match conn.prepare(
            "SELECT id, title, reminder_time 
             FROM tasks 
             WHERE reminder_time IS NOT NULL 
               AND due_date = ?1 
               AND status = 'needsAction' 
               AND is_deleted = 0"
        ) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to prepare reminder query: {}", e);
                continue;
            }
        };

        let tasks_res = stmt.query_map([today_str], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let time_str: String = row.get(2)?;
            let reminder_time = NaiveTime::parse_from_str(&time_str, "%H:%M:%S")
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(ReminderTask { id, title, reminder_time })
        });

        match tasks_res {
            Ok(mapped_rows) => {
                for task_opt in mapped_rows {
                    if let Ok(task) = task_opt {
                        if current_time >= task.reminder_time {
                            if !notified_tasks.contains(&task.id) {
                                info!("Triggering desktop notification for task '{}' (due at {})", task.title, task.reminder_time);
                                let notification_res = Notification::new()
                                    .summary("TaskFlow Reminder")
                                    .body(&task.title)
                                    .appname("TaskFlow")
                                    .icon("taskflow")
                                    .timeout(0) // Persistent until dismissed
                                    .show();

                                if let Err(e) = notification_res {
                                    error!("Failed to display notification: {}", e);
                                } else {
                                    notified_tasks.insert(task.id);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to execute reminder tasks query: {}", e);
            }
        }
    }
}

async fn run_sync_loop(db: Database) {
    info!("Starting background sync loop");

    loop {
        // Sleep for 5 minutes between sync runs
        sleep(Duration::from_secs(300)).await;

        let token_manager = TokenManager::new();
        if !token_manager.has_refresh_token() {
            warn!("Sync loop: User is not authenticated. Skipping sync.");
            continue;
        }

        info!("Starting background sync with Google Tasks");

        let creds = match load_credentials() {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to load credentials for sync: {}", e);
                continue;
            }
        };

        let mut client = GoogleTasksClient::new(creds, token_manager);
        match run_sync(&db, &mut client).await {
            Ok(report) => {
                info!(
                    "Sync finished successfully. Pulled tasks: {}, Pushed tasks: {}, Deleted tasks: {}",
                    report.tasks_pulled, report.tasks_pushed, report.tasks_deleted
                );
            }
            Err(e) => {
                error!("Sync loop execution failed: {}", e);
            }
        }
    }
}
