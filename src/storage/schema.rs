use rusqlite::Connection;

/// Run all database migrations. Called on every open.
pub fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS media_roots (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            position INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS watch_history (
            filename TEXT PRIMARY KEY,
            directory TEXT NOT NULL,
            last_watched INTEGER,
            watch_count INTEGER NOT NULL DEFAULT 0,
            last_position REAL NOT NULL DEFAULT 0.0,
            completed INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}
