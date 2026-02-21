use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

use dessplay_core::protocol::{CrdtOp, CrdtSnapshot};
use dessplay_core::types::FileId;

// ---------------------------------------------------------------------------
// AniDB revalidation schedule constants (milliseconds)
// ---------------------------------------------------------------------------

const THIRTY_MINUTES: u64 = 30 * 60 * 1000;
const TWO_HOURS: u64 = 2 * 60 * 60 * 1000;
const ONE_DAY: u64 = 24 * 60 * 60 * 1000;
const ONE_WEEK: u64 = 7 * ONE_DAY;
const THREE_MONTHS: u64 = 90 * ONE_DAY;

// ---------------------------------------------------------------------------
// Server storage
// ---------------------------------------------------------------------------

pub struct ServerStorage {
    conn: Connection,
}

impl ServerStorage {
    /// Open (or create) the database at the given path and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    // -----------------------------------------------------------------------
    // Migrations
    // -----------------------------------------------------------------------

    fn migrate(&self) -> Result<()> {
        let version = self.schema_version()?;
        if version < 1 {
            self.migrate_v1()?;
        }
        Ok(())
    }

    fn schema_version(&self) -> Result<u32> {
        let table_exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='metadata'",
            [],
            |row| row.get(0),
        )?;
        if !table_exists {
            return Ok(0);
        }
        let version: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match version {
            Some(v) => Ok(v.parse::<u32>().context("invalid schema_version in metadata")?),
            None => Ok(0),
        }
    }

    fn migrate_v1(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS crdt_snapshots (
                epoch      INTEGER PRIMARY KEY,
                data       BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS crdt_ops (
                id    INTEGER PRIMARY KEY AUTOINCREMENT,
                epoch INTEGER NOT NULL,
                data  BLOB NOT NULL
            );

            CREATE TABLE IF NOT EXISTS anidb_queue (
                file_hash       BLOB PRIMARY KEY,
                has_data        INTEGER NOT NULL DEFAULT 0,
                first_seen_at   INTEGER NOT NULL,
                last_checked_at INTEGER,
                next_check_at   INTEGER NOT NULL,
                retry_count     INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', '1')",
            [],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // CRDT persistence
    // -----------------------------------------------------------------------

    pub fn save_snapshot(&self, epoch: u64, snapshot: &CrdtSnapshot) -> Result<()> {
        let data = postcard::to_allocvec(snapshot).context("failed to serialize CRDT snapshot")?;
        self.conn.execute(
            "INSERT OR REPLACE INTO crdt_snapshots (epoch, data, created_at) VALUES (?1, ?2, ?3)",
            params![epoch as i64, data, now_millis()],
        )?;
        Ok(())
    }

    pub fn load_latest_snapshot(&self) -> Result<Option<(u64, CrdtSnapshot)>> {
        let row: Option<(i64, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT epoch, data FROM crdt_snapshots ORDER BY epoch DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match row {
            Some((epoch, data)) => {
                let snapshot: CrdtSnapshot =
                    postcard::from_bytes(&data).context("failed to deserialize CRDT snapshot")?;
                Ok(Some((epoch as u64, snapshot)))
            }
            None => Ok(None),
        }
    }

    pub fn append_op(&self, epoch: u64, op: &CrdtOp) -> Result<()> {
        let data = postcard::to_allocvec(op).context("failed to serialize CRDT op")?;
        self.conn.execute(
            "INSERT INTO crdt_ops (epoch, data) VALUES (?1, ?2)",
            params![epoch as i64, data],
        )?;
        Ok(())
    }

    pub fn load_ops(&self, epoch: u64) -> Result<Vec<CrdtOp>> {
        let mut stmt = self
            .conn
            .prepare("SELECT data FROM crdt_ops WHERE epoch = ?1 ORDER BY id")?;
        let blobs = stmt
            .query_map(params![epoch as i64], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        blobs
            .iter()
            .map(|data| postcard::from_bytes(data).context("failed to deserialize CRDT op"))
            .collect()
    }

    pub fn delete_before_epoch(&self, epoch: u64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM crdt_snapshots WHERE epoch < ?1",
            params![epoch as i64],
        )?;
        self.conn.execute(
            "DELETE FROM crdt_ops WHERE epoch < ?1",
            params![epoch as i64],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // AniDB validation queue
    // -----------------------------------------------------------------------

    /// Add a file to the AniDB lookup queue. No-op if already queued.
    pub fn enqueue_anidb_lookup(&self, file_id: &FileId, now: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO anidb_queue \
             (file_hash, has_data, first_seen_at, next_check_at, retry_count) \
             VALUES (?1, 0, ?2, ?2, 0)",
            params![file_id.0.as_slice(), now as i64],
        )?;
        Ok(())
    }

    /// Get the next file due for AniDB lookup (earliest `next_check_at` <= now).
    pub fn get_next_pending(&self, now: u64) -> Result<Option<FileId>> {
        let row: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT file_hash FROM anidb_queue \
                 WHERE next_check_at <= ?1 \
                 ORDER BY next_check_at ASC LIMIT 1",
                params![now as i64],
                |row| row.get(0),
            )
            .optional()?;
        match row {
            Some(hash) => {
                ensure!(
                    hash.len() == 16,
                    "corrupt file_hash in anidb_queue: expected 16 bytes, got {}",
                    hash.len()
                );
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&hash);
                Ok(Some(FileId(arr)))
            }
            None => Ok(None),
        }
    }

    /// Record a successful AniDB lookup. Schedules revalidation per design rules.
    pub fn record_success(&self, file_id: &FileId, now: u64) -> Result<()> {
        let next = now + ONE_WEEK;
        self.conn.execute(
            "UPDATE anidb_queue \
             SET has_data = 1, last_checked_at = ?1, next_check_at = ?2, retry_count = 0 \
             WHERE file_hash = ?3",
            params![now as i64, next as i64, file_id.0.as_slice()],
        )?;
        Ok(())
    }

    /// Record a failed AniDB lookup (no data returned). Schedules revalidation
    /// based on file age per design rules.
    pub fn record_failure(&self, file_id: &FileId, now: u64) -> Result<()> {
        let row: Option<(i64, i32)> = self
            .conn
            .query_row(
                "SELECT first_seen_at, retry_count FROM anidb_queue WHERE file_hash = ?1",
                params![file_id.0.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((first_seen, retry_count)) = row else {
            return Ok(());
        };
        let age = now.saturating_sub(first_seen as u64);
        let interval = compute_recheck_interval(age);
        let next = now + interval;
        self.conn.execute(
            "UPDATE anidb_queue \
             SET last_checked_at = ?1, next_check_at = ?2, retry_count = ?3 \
             WHERE file_hash = ?4",
            params![
                now as i64,
                next as i64,
                retry_count + 1,
                file_id.0.as_slice()
            ],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Default DB path
// ---------------------------------------------------------------------------

pub fn default_db_path() -> Result<PathBuf> {
    let data_dir = std::env::var("DESSPLAY_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    Ok(data_dir.join("dessplay-rendezvous.db"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Compute the recheck interval for a file that AniDB has no data for,
/// based on how old the file entry is.
///
/// Per design.md:
/// - < 1 day old: every 30 minutes
/// - < 1 week old: every 2 hours
/// - < 3 months old: every day
/// - >= 3 months: stop rechecking (use a very large interval)
fn compute_recheck_interval(age_ms: u64) -> u64 {
    if age_ms < ONE_DAY {
        THIRTY_MINUTES
    } else if age_ms < ONE_WEEK {
        TWO_HOURS
    } else if age_ms < THREE_MONTHS {
        ONE_DAY
    } else {
        // Effectively never — 10 years
        365 * 10 * ONE_DAY
    }
}

/// Extension trait for converting `QueryReturnedNoRows` into `Ok(None)`.
trait OptionalRow<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalRow<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use dessplay_core::protocol::{CrdtOp, LwwValue, PlaylistAction};
    use dessplay_core::types::{FileId, UserState};

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    fn uid(name: &str) -> dessplay_core::types::UserId {
        dessplay_core::types::UserId(name.to_string())
    }

    #[test]
    fn migration_creates_tables() {
        let db = ServerStorage::open_in_memory().unwrap();
        let tables: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(tables.contains(&"metadata".to_string()));
        assert!(tables.contains(&"crdt_snapshots".to_string()));
        assert!(tables.contains(&"crdt_ops".to_string()));
        assert!(tables.contains(&"anidb_queue".to_string()));
    }

    #[test]
    fn migration_is_idempotent() {
        let db = ServerStorage::open_in_memory().unwrap();
        db.migrate().unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    #[test]
    fn snapshot_round_trip() {
        let db = ServerStorage::open_in_memory().unwrap();
        let mut state = dessplay_core::crdt::CrdtState::new();
        state.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::UserState(uid("alice"), UserState::Ready),
        });
        state.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 200,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        });

        let snapshot = state.snapshot();
        db.save_snapshot(1, &snapshot).unwrap();

        let (epoch, loaded) = db.load_latest_snapshot().unwrap().unwrap();
        assert_eq!(epoch, 1);
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn op_log_round_trip() {
        let db = ServerStorage::open_in_memory().unwrap();
        let ops = vec![
            CrdtOp::LwwWrite {
                timestamp: 10,
                value: LwwValue::UserState(uid("alice"), UserState::Ready),
            },
            CrdtOp::ChatAppend {
                user_id: uid("bob"),
                seq: 0,
                timestamp: 30,
                text: "hi".into(),
            },
        ];
        for op in &ops {
            db.append_op(1, op).unwrap();
        }
        let loaded = db.load_ops(1).unwrap();
        assert_eq!(loaded.len(), 2);

        let mut original = dessplay_core::crdt::CrdtState::new();
        for op in &ops {
            original.apply_op(op);
        }
        let mut replayed = dessplay_core::crdt::CrdtState::new();
        for op in &loaded {
            replayed.apply_op(op);
        }
        assert_eq!(original.snapshot(), replayed.snapshot());
    }

    #[test]
    fn epoch_cleanup() {
        let db = ServerStorage::open_in_memory().unwrap();
        let state = dessplay_core::crdt::CrdtState::new();
        db.save_snapshot(1, &state.snapshot()).unwrap();
        db.save_snapshot(2, &state.snapshot()).unwrap();
        db.save_snapshot(3, &state.snapshot()).unwrap();
        db.append_op(
            1,
            &CrdtOp::LwwWrite {
                timestamp: 1,
                value: LwwValue::UserState(uid("a"), UserState::Ready),
            },
        )
        .unwrap();
        db.append_op(
            3,
            &CrdtOp::LwwWrite {
                timestamp: 2,
                value: LwwValue::UserState(uid("b"), UserState::Ready),
            },
        )
        .unwrap();

        db.delete_before_epoch(3).unwrap();

        let (epoch, _) = db.load_latest_snapshot().unwrap().unwrap();
        assert_eq!(epoch, 3);
        assert!(db.load_ops(1).unwrap().is_empty());
        assert_eq!(db.load_ops(3).unwrap().len(), 1);
    }

    // -----------------------------------------------------------------------
    // AniDB queue tests
    // -----------------------------------------------------------------------

    #[test]
    fn anidb_enqueue_and_get_pending() {
        let db = ServerStorage::open_in_memory().unwrap();
        let f1 = fid(1);
        let now = 1_000_000;

        // Nothing pending initially
        assert!(db.get_next_pending(now).unwrap().is_none());

        // Enqueue a file — next_check_at = now, so it's immediately pending
        db.enqueue_anidb_lookup(&f1, now).unwrap();
        assert_eq!(db.get_next_pending(now).unwrap(), Some(f1));
    }

    #[test]
    fn anidb_enqueue_is_idempotent() {
        let db = ServerStorage::open_in_memory().unwrap();
        let f1 = fid(1);
        db.enqueue_anidb_lookup(&f1, 1000).unwrap();
        // Re-enqueue with a different timestamp should be a no-op (INSERT OR IGNORE)
        db.enqueue_anidb_lookup(&f1, 9999).unwrap();

        // Should still use the original timestamp
        assert!(db.get_next_pending(1000).unwrap().is_some());
    }

    #[test]
    fn anidb_success_schedules_weekly_recheck() {
        let db = ServerStorage::open_in_memory().unwrap();
        let f1 = fid(1);
        let now = 1_000_000;

        db.enqueue_anidb_lookup(&f1, now).unwrap();
        db.record_success(&f1, now).unwrap();

        // Should not be pending now
        assert!(db.get_next_pending(now).unwrap().is_none());

        // Should be pending after one week
        assert!(db.get_next_pending(now + ONE_WEEK).unwrap().is_some());
    }

    #[test]
    fn anidb_failure_schedules_based_on_age() {
        let db = ServerStorage::open_in_memory().unwrap();
        let f1 = fid(1);
        let now = 1_000_000;

        db.enqueue_anidb_lookup(&f1, now).unwrap();
        db.record_failure(&f1, now).unwrap();

        // File is < 1 day old, so next check is in 30 minutes
        assert!(db.get_next_pending(now).unwrap().is_none());
        assert!(db
            .get_next_pending(now + THIRTY_MINUTES)
            .unwrap()
            .is_some());
    }

    #[test]
    fn anidb_failure_age_tiers() {
        let db = ServerStorage::open_in_memory().unwrap();
        let f1 = fid(1);

        // File first seen at t=0
        let first_seen = 0u64;
        db.enqueue_anidb_lookup(&f1, first_seen).unwrap();

        // Check at 2 days old → < 1 week → next check in 2 hours
        let check_time = 2 * ONE_DAY;
        db.record_failure(&f1, check_time).unwrap();
        assert!(db.get_next_pending(check_time).unwrap().is_none());
        assert!(db
            .get_next_pending(check_time + TWO_HOURS)
            .unwrap()
            .is_some());

        // Check at 2 weeks old → < 3 months → next check in 1 day
        let check_time_2 = 14 * ONE_DAY;
        db.record_failure(&f1, check_time_2).unwrap();
        assert!(db.get_next_pending(check_time_2).unwrap().is_none());
        assert!(db
            .get_next_pending(check_time_2 + ONE_DAY)
            .unwrap()
            .is_some());

        // Check at 4 months old → >= 3 months → effectively never
        let check_time_3 = 120 * ONE_DAY;
        db.record_failure(&f1, check_time_3).unwrap();
        assert!(db.get_next_pending(check_time_3).unwrap().is_none());
        assert!(db
            .get_next_pending(check_time_3 + ONE_DAY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn anidb_pending_order_by_next_check() {
        let db = ServerStorage::open_in_memory().unwrap();
        let f1 = fid(1);
        let f2 = fid(2);

        db.enqueue_anidb_lookup(&f1, 2000).unwrap();
        db.enqueue_anidb_lookup(&f2, 1000).unwrap();

        // f2 has earlier next_check_at, should come first
        let next = db.get_next_pending(3000).unwrap().unwrap();
        assert_eq!(next, f2);
    }

    #[test]
    fn no_snapshot_returns_none() {
        let db = ServerStorage::open_in_memory().unwrap();
        assert!(db.load_latest_snapshot().unwrap().is_none());
    }
}
