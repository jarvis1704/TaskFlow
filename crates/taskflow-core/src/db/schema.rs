use rusqlite::Connection;

pub fn init_db(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Enable foreign key constraints
    conn.pragma_update(None, "foreign_keys", "ON")?;
    
    // Enable WAL mode for safe concurrent access between GUI and daemon processes
    conn.pragma_update(None, "journal_mode", "WAL")?;

    // Create task_lists table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS task_lists (
            id          TEXT PRIMARY KEY,
            google_id   TEXT UNIQUE,
            title       TEXT NOT NULL,
            position    INTEGER NOT NULL DEFAULT 0,
            is_default  INTEGER NOT NULL DEFAULT 0,
            updated_at  TEXT NOT NULL
        );",
        [],
    )?;

    // Create tasks table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tasks (
            id                  TEXT PRIMARY KEY,
            google_id           TEXT UNIQUE,
            list_id             TEXT NOT NULL REFERENCES task_lists(id) ON DELETE CASCADE,
            title               TEXT NOT NULL,
            notes               TEXT,
            status              TEXT NOT NULL DEFAULT 'needsAction',
            due_date            TEXT,
            reminder_time       TEXT,
            parent_id           TEXT REFERENCES tasks(id) ON DELETE SET NULL,
            position            TEXT,
            completed_at        TEXT,
            updated_at          TEXT NOT NULL,
            google_updated_at   TEXT,
            sync_state          TEXT NOT NULL DEFAULT 'pending',
            is_deleted          INTEGER NOT NULL DEFAULT 0,
            recurrence_rule     TEXT,
            starred             INTEGER NOT NULL DEFAULT 0
        );",
        [],
    )?;

    // Create indexes for performance
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tasks_list_id ON tasks(list_id);",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tasks_parent_id ON tasks(parent_id);",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tasks_due_date ON tasks(due_date);",
        [],
    )?;

    // Self-healing migration for existing databases: Check if starred column exists, if not, add it
    {
        let mut stmt = conn.prepare("PRAGMA table_info(tasks)")?;
        let mut has_starred = false;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == "starred" {
                has_starred = true;
                break;
            }
        }
        if !has_starred {
            conn.execute("ALTER TABLE tasks ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;", [])?;
        }
    }

    // Create sync_meta table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sync_meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );",
        [],
    )?;

    Ok(())
}
