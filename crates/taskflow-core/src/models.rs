use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    Pending,
    Synced,
    Conflict,
    DeletedPending,
}

impl std::fmt::Display for SyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SyncState::Pending => "pending",
            SyncState::Synced => "synced",
            SyncState::Conflict => "conflict",
            SyncState::DeletedPending => "deleted_pending",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for SyncState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(SyncState::Pending),
            "synced" => Ok(SyncState::Synced),
            "conflict" => Ok(SyncState::Conflict),
            "deleted_pending" => Ok(SyncState::DeletedPending),
            _ => Err(format!("Unknown sync state: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum RecurrenceRule {
    Daily,
    Weekly(chrono::Weekday),
    Monthly(u32), // Day of month (1-31)
    Custom(String), // Custom RRULE-style string
}

impl std::fmt::Display for RecurrenceRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecurrenceRule::Daily => write!(f, "Daily"),
            RecurrenceRule::Weekly(wd) => write!(f, "Weekly on {:?}", wd),
            RecurrenceRule::Monthly(day) => write!(f, "Monthly on day {}", day),
            RecurrenceRule::Custom(rrule) => write!(f, "Custom: {}", rrule),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    pub id: String, // Local UUID
    pub google_id: Option<String>,
    pub title: String,
    pub position: i32,
    pub is_default: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String, // Local UUID
    pub google_id: Option<String>,
    pub list_id: String,
    pub title: String,
    pub notes: Option<String>,
    pub status: String, // "needsAction" | "completed"
    pub due_date: Option<NaiveDate>,
    pub reminder_time: Option<NaiveTime>,
    pub parent_id: Option<String>, // Self-referencing UUID for subtasks
    pub position: Option<String>, // Google Tasks lexicographic position
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub google_updated_at: Option<DateTime<Utc>>,
    pub sync_state: SyncState,
    pub is_deleted: bool,
    pub recurrence_rule: Option<RecurrenceRule>,
}
