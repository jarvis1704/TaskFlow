use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    UseLocal,
    UseRemote,
}

/// Resolves a conflict based on the most-recent-write-wins (MRWW) strategy.
pub fn resolve_conflict(local_updated: DateTime<Utc>, remote_updated: DateTime<Utc>) -> ConflictResolution {
    if local_updated >= remote_updated {
        ConflictResolution::UseLocal
    } else {
        ConflictResolution::UseRemote
    }
}
