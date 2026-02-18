pub mod schema;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use self::schema::run_migrations;

/// A media directory entry from the recent_series query.
pub struct RecentSeriesEntry {
    pub directory: String,
    pub last_watched: i64,
}

/// Local persistence layer backed by SQLite.
///
/// All methods are synchronous (operations are fast <1ms queries).
/// The `Mutex<Connection>` is never held across await points.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open or create the database at the given path. Runs migrations.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database (for testing).
    #[cfg(test)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // -- Media Roots --

    /// Add a media root directory. Appends at the end of the list.
    /// Returns an error if the path already exists.
    pub fn add_media_root(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let max_pos: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM media_roots",
                [],
                |row| row.get(0),
            )
            .unwrap_or(-1);
        conn.execute(
            "INSERT INTO media_roots (path, position) VALUES (?1, ?2)",
            rusqlite::params![path, max_pos + 1],
        )?;
        Ok(())
    }

    /// Remove a media root directory. Returns true if it existed.
    pub fn remove_media_root(&self, path: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute("DELETE FROM media_roots WHERE path = ?1", [path])?;
        Ok(changed > 0)
    }

    /// List all media root directories in order.
    pub fn list_media_roots(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT path FROM media_roots ORDER BY position")?;
        let roots = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(roots)
    }

    // -- Watch History --

    /// Record playback progress for a file. Creates or updates the entry.
    pub fn record_watch_progress(
        &self,
        filename: &str,
        directory: &str,
        position: f64,
        duration: f64,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        conn.execute(
            "INSERT INTO watch_history (filename, directory, last_watched, watch_count, last_position, completed)
             VALUES (?1, ?2, ?3, 1, ?4, 0)
             ON CONFLICT(filename) DO UPDATE SET
               last_watched = ?3,
               last_position = ?4,
               watch_count = watch_count + 1",
            rusqlite::params![filename, directory, now, position],
        )?;
        // Auto-mark watched at 90% threshold
        if duration > 0.0 && position / duration >= 0.9 {
            conn.execute(
                "UPDATE watch_history SET completed = 1 WHERE filename = ?1",
                [filename],
            )?;
        }
        Ok(())
    }

    /// Explicitly mark a file as watched.
    pub fn mark_watched(&self, filename: &str, directory: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        conn.execute(
            "INSERT INTO watch_history (filename, directory, last_watched, watch_count, last_position, completed)
             VALUES (?1, ?2, ?3, 1, 0.0, 1)
             ON CONFLICT(filename) DO UPDATE SET
               last_watched = ?3,
               completed = 1",
            rusqlite::params![filename, directory, now],
        )?;
        Ok(())
    }

    /// Check whether a file has been watched (completed = true).
    pub fn is_watched(&self, filename: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result: Option<bool> = conn
            .query_row(
                "SELECT completed FROM watch_history WHERE filename = ?1",
                [filename],
                |row| row.get(0),
            )
            .ok();
        Ok(result.unwrap_or(false))
    }

    /// Return directories with watch history, sorted by most recently watched first.
    /// Only returns directories that have at least one watched file.
    pub fn recent_series(&self) -> anyhow::Result<Vec<RecentSeriesEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT directory, MAX(last_watched) as most_recent
             FROM watch_history
             WHERE last_watched IS NOT NULL
             GROUP BY directory
             ORDER BY most_recent DESC",
        )?;
        let entries = stmt
            .query_map([], |row| {
                Ok(RecentSeriesEntry {
                    directory: row.get(0)?,
                    last_watched: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// Check if a directory is a "known series" (has any watch history).
    pub fn is_known_series(&self, directory: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM watch_history WHERE directory = ?1",
            [directory],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_roots_round_trip() {
        let db = Database::open_in_memory().unwrap();

        // Initially empty
        assert!(db.list_media_roots().unwrap().is_empty());

        // Add roots
        db.add_media_root("/home/user/anime").unwrap();
        db.add_media_root("/mnt/nas/shows").unwrap();

        let roots = db.list_media_roots().unwrap();
        assert_eq!(roots, vec!["/home/user/anime", "/mnt/nas/shows"]);

        // Remove one
        assert!(db.remove_media_root("/home/user/anime").unwrap());
        let roots = db.list_media_roots().unwrap();
        assert_eq!(roots, vec!["/mnt/nas/shows"]);

        // Remove non-existent
        assert!(!db.remove_media_root("/nonexistent").unwrap());
    }

    #[test]
    fn media_roots_duplicate_rejected() {
        let db = Database::open_in_memory().unwrap();
        db.add_media_root("/home/user/anime").unwrap();
        assert!(db.add_media_root("/home/user/anime").is_err());
    }

    #[test]
    fn watch_history_basics() {
        let db = Database::open_in_memory().unwrap();

        // Not watched initially
        assert!(!db.is_watched("ep01.mkv").unwrap());

        // Record progress below 90%
        db.record_watch_progress("ep01.mkv", "/anime/frieren", 100.0, 1400.0)
            .unwrap();
        assert!(!db.is_watched("ep01.mkv").unwrap());

        // Record progress above 90%
        db.record_watch_progress("ep01.mkv", "/anime/frieren", 1300.0, 1400.0)
            .unwrap();
        assert!(db.is_watched("ep01.mkv").unwrap());
    }

    #[test]
    fn mark_watched_explicit() {
        let db = Database::open_in_memory().unwrap();
        db.mark_watched("ep02.mkv", "/anime/frieren").unwrap();
        assert!(db.is_watched("ep02.mkv").unwrap());
    }

    #[test]
    fn recent_series_ordering() {
        let db = Database::open_in_memory().unwrap();

        // Watch files from two different series (different filenames!)
        db.mark_watched("frieren-01.mkv", "/anime/frieren").unwrap();
        // Small delay to ensure different timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));
        db.mark_watched("oshi-01.mkv", "/anime/oshi-no-ko").unwrap();

        let series = db.recent_series().unwrap();
        assert_eq!(series.len(), 2);
        // Most recently watched first
        assert_eq!(series[0].directory, "/anime/oshi-no-ko");
        assert_eq!(series[1].directory, "/anime/frieren");
    }

    #[test]
    fn is_known_series() {
        let db = Database::open_in_memory().unwrap();

        assert!(!db.is_known_series("/anime/frieren").unwrap());

        db.record_watch_progress("ep01.mkv", "/anime/frieren", 10.0, 1400.0)
            .unwrap();
        assert!(db.is_known_series("/anime/frieren").unwrap());
        assert!(!db.is_known_series("/anime/other").unwrap());
    }
}
