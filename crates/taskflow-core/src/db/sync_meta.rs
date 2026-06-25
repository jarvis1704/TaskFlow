use rusqlite::{params, Connection, OptionalExtension};

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT value FROM sync_meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO sync_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, key: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM sync_meta WHERE key = ?1", params![key])?;
    Ok(())
}
