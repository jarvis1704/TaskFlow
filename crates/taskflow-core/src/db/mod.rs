pub mod schema;
pub mod sync_meta;
pub mod task_lists;
pub mod tasks;

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::fs;
use directories::ProjectDirs;

#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
}

impl Database {
    /// Create a new Database manager pointing to the XDG default data directory
    pub fn new() -> Result<Self, String> {
        let proj_dirs = ProjectDirs::from("org", "taskflow", "taskflow")
            .ok_or_else(|| "Could not determine XDG directories".to_string())?;
        
        let data_dir = proj_dirs.data_dir();
        fs::create_dir_all(data_dir)
            .map_err(|e| format!("Failed to create data directory {:?}: {}", data_dir, e))?;

        let db_path = data_dir.join("taskflow.db");
        Ok(Self { path: db_path })
    }

    /// Create an in-memory database for testing
    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::from(":memory:"),
        }
    }

    /// Create a database manager at a custom path
    pub fn at_path<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Get the path to the database file
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open a new connection and ensure the database schema is initialized
    pub fn connect(&self) -> Result<Connection, rusqlite::Error> {
        let conn = if self.path.to_string_lossy() == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(&self.path)?
        };

        // Initialize the schema (tables, constraints, WAL mode)
        schema::init_db(&conn)?;

        Ok(conn)
    }
}
