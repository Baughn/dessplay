//! Server-side SQLite persistence.
//!
//! One database (default `$XDG_DATA_HOME/dessplay-rendezvous/rendezvous.db`,
//! overridable via `--db-path`) holds the authoritative CRDT snapshot, the
//! full chat archive (compaction trims the replicated chat to 500 messages
//! after archiving here), and the AniDB validation queue. Scheduling logic
//! for the queue arrives in Phase 8; this module only stores it.
//!
//! Timestamps are caller-supplied unix milliseconds (`i64`); storage never
//! reads the clock.

// Consumed by the server actor in Phase 5; only tests use it until then.
#![allow(dead_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use dessplay_core::types::{ChatMessage, Ed2kHash, Epoch, FileHashInfo, SharedTimestamp, UserId};
use dessplay_core::wire::WireError;
use dessplay_core::{CrdtState, StateSnapshot, wire};
use rusqlite::{Connection, OptionalExtension, params};

/// Storage errors. SQLite failures, snapshot (de)serialization failures,
/// or corrupt rows.
#[derive(Debug)]
pub enum StorageError {
    /// Underlying SQLite error.
    Sqlite(rusqlite::Error),
    /// Postcard encode/decode failure on a snapshot blob.
    Codec(WireError),
    /// A stored value failed validation.
    Corrupt(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Sqlite(e) => write!(f, "sqlite error: {e}"),
            StorageError::Codec(e) => write!(f, "snapshot codec error: {e}"),
            StorageError::Corrupt(what) => write!(f, "corrupt storage: {what}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Sqlite(e)
    }
}

impl From<WireError> for StorageError {
    fn from(e: WireError) -> Self {
        StorageError::Codec(e)
    }
}

/// Result alias for storage operations.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Versioned schema; `PRAGMA user_version` records progress. Append-only.
const MIGRATIONS: &[&str] = &[
    // v1: initial schema.
    "
    CREATE TABLE crdt_state (
        room     TEXT PRIMARY KEY,     -- single implicit room in v1
        epoch    INTEGER NOT NULL,
        state    BLOB NOT NULL,        -- postcard CrdtState
        saved_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE chat_archive (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp INTEGER NOT NULL,    -- shared-clock millis
        sender    TEXT NOT NULL,
        text      TEXT NOT NULL,
        UNIQUE (timestamp, sender, text)  -- mirrors GList dedup semantics
    ) STRICT;
    CREATE INDEX chat_archive_timestamp ON chat_archive (timestamp);

    CREATE TABLE anidb_queue (
        hash         BLOB PRIMARY KEY, -- 16-byte ed2k root
        size_bytes   INTEGER NOT NULL, -- AniDB FILE needs (hash, size)
        filename     TEXT NOT NULL,
        first_seen   INTEGER NOT NULL,
        last_attempt INTEGER,          -- NULL = never tried
        next_attempt INTEGER NOT NULL, -- when to (re)try; Phase 8 schedules
        attempts     INTEGER NOT NULL DEFAULT 0,
        has_data     INTEGER NOT NULL DEFAULT 0  -- AniDB knew the file
    ) STRICT;
    CREATE INDEX anidb_queue_next_attempt ON anidb_queue (next_attempt);
    ",
];

/// Apply any unapplied migrations (slice parameter for upgrade tests).
fn migrate(conn: &Connection, migrations: &[&str]) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let applied = usize::try_from(version)
        .map_err(|_| StorageError::Corrupt(format!("negative user_version {version}")))?;
    if applied > migrations.len() {
        return Err(StorageError::Corrupt(format!(
            "database is from the future: user_version {applied} > {} known migrations",
            migrations.len()
        )));
    }
    for (index, migration) in migrations.iter().enumerate().skip(applied) {
        conn.execute_batch(&format!(
            "BEGIN;\n{migration}\nPRAGMA user_version = {};\nCOMMIT;",
            index + 1
        ))?;
    }
    Ok(())
}

/// One AniDB validation-queue row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    /// What to look up.
    pub info: FileHashInfo,
    /// When the request first appeared.
    pub first_seen: i64,
    /// Last attempt time; `None` = never tried.
    pub last_attempt: Option<i64>,
    /// Earliest time of the next attempt.
    pub next_attempt: i64,
    /// How many attempts so far.
    pub attempts: u32,
    /// Whether AniDB has ever returned data for this file (drives the
    /// gentler re-validation cadence).
    pub has_data: bool,
}

fn hash_from_blob(blob: Vec<u8>) -> Result<Ed2kHash> {
    let bytes: [u8; 16] = blob
        .try_into()
        .map_err(|blob: Vec<u8>| StorageError::Corrupt(format!("hash blob len {}", blob.len())))?;
    Ok(Ed2kHash(bytes))
}

/// The rendezvous server's persistent storage.
pub struct ServerStorage {
    conn: Connection,
}

impl ServerStorage {
    /// Open (creating and migrating as needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::Corrupt(format!("creating {parent:?}: {e}")))?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// An in-memory database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&conn, MIGRATIONS)?;
        Ok(Self { conn })
    }

    /// The default database path:
    /// `$XDG_DATA_HOME/dessplay-rendezvous/rendezvous.db`.
    pub fn default_path() -> Option<PathBuf> {
        Some(
            dirs::data_dir()?
                .join("dessplay-rendezvous")
                .join("rendezvous.db"),
        )
    }

    // ---- CRDT snapshot.

    /// Persist the authoritative snapshot (single implicit room).
    pub fn save_state(&self, snapshot: &StateSnapshot, now: i64) -> Result<()> {
        let blob = wire::encode(&snapshot.state)?;
        self.conn.execute(
            "INSERT INTO crdt_state (room, epoch, state, saved_at)
             VALUES ('default', ?1, ?2, ?3)
             ON CONFLICT (room) DO UPDATE
             SET epoch = excluded.epoch, state = excluded.state,
                 saved_at = excluded.saved_at",
            params![snapshot.epoch.0 as i64, blob, now],
        )?;
        Ok(())
    }

    /// Load the stored snapshot, if any.
    pub fn load_state(&self) -> Result<Option<StateSnapshot>> {
        let row: Option<(i64, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT epoch, state FROM crdt_state WHERE room = 'default'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((epoch, blob)) = row else {
            return Ok(None);
        };
        let state: CrdtState = wire::decode(&blob)?;
        Ok(Some(StateSnapshot {
            epoch: Epoch(epoch as u64),
            state,
        }))
    }

    // ---- Chat archive.

    /// Archive messages (idempotent: re-archiving already-stored messages
    /// is a no-op, mirroring GList dedup). Returns how many were new.
    pub fn archive_chat(&mut self, messages: &[ChatMessage]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut new = 0;
        for message in messages {
            new += tx.execute(
                "INSERT OR IGNORE INTO chat_archive (timestamp, sender, text)
                 VALUES (?1, ?2, ?3)",
                params![message.timestamp.0 as i64, message.sender.0, message.text],
            )?;
        }
        tx.commit()?;
        Ok(new)
    }

    /// Read archived messages in chronological order, optionally only
    /// those at or after `since` (shared-clock millis).
    pub fn chat_archive(&self, since: Option<i64>, limit: usize) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, sender, text FROM chat_archive
             WHERE timestamp >= ?1 ORDER BY timestamp, id LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![since.unwrap_or(i64::MIN), limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            let (timestamp, sender, text) = row?;
            messages.push(ChatMessage {
                timestamp: SharedTimestamp(timestamp as u64),
                sender: UserId(sender),
                text,
            });
        }
        Ok(messages)
    }

    // ---- AniDB validation queue.

    /// Add a lookup request if it isn't already queued. New entries are
    /// due immediately (`next_attempt = now`).
    pub fn enqueue_lookup(&self, info: &FileHashInfo, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO anidb_queue
             (hash, size_bytes, filename, first_seen, next_attempt)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![info.hash.0.as_slice(), info.size as i64, info.filename, now],
        )?;
        Ok(())
    }

    /// Entries due at or before `now`, soonest first.
    pub fn due_lookups(&self, now: i64, limit: usize) -> Result<Vec<QueueEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, size_bytes, filename, first_seen, last_attempt,
                    next_attempt, attempts, has_data
             FROM anidb_queue WHERE next_attempt <= ?1
             ORDER BY next_attempt, hash LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now, limit as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (blob, size, filename, first_seen, last_attempt, next_attempt, attempts, has_data) =
                row?;
            entries.push(QueueEntry {
                info: FileHashInfo {
                    hash: hash_from_blob(blob)?,
                    size: size as u64,
                    filename,
                },
                first_seen,
                last_attempt,
                next_attempt,
                attempts: attempts as u32,
                has_data: has_data != 0,
            });
        }
        Ok(entries)
    }

    /// Record an attempt and its next scheduled time. `got_data` marks
    /// whether AniDB returned data (now or ever before).
    pub fn record_lookup_attempt(
        &self,
        hash: Ed2kHash,
        attempted_at: i64,
        next_attempt: i64,
        got_data: bool,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE anidb_queue
             SET last_attempt = ?2, next_attempt = ?3, attempts = attempts + 1,
                 has_data = max(has_data, ?4)
             WHERE hash = ?1",
            params![
                hash.0.as_slice(),
                attempted_at,
                next_attempt,
                got_data as i64
            ],
        )?;
        Ok(())
    }

    /// Drop a queue entry (file no longer needs validation).
    pub fn remove_lookup(&self, hash: Ed2kHash) -> Result<()> {
        self.conn.execute(
            "DELETE FROM anidb_queue WHERE hash = ?1",
            params![hash.0.as_slice()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::test_support::{ClusterEvent, ScriptOp, run_cluster};

    use super::*;

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn msg(t: u64, who: &str, text: &str) -> ChatMessage {
        ChatMessage {
            timestamp: SharedTimestamp(t),
            sender: UserId::new(who),
            text: text.into(),
        }
    }

    #[test]
    fn migrations_apply_and_reject_future() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn, MIGRATIONS).unwrap();
        migrate(&conn, MIGRATIONS).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(migrate(&conn, MIGRATIONS).is_err());
    }

    #[test]
    fn snapshot_round_trips() {
        let storage = ServerStorage::open_in_memory().unwrap();
        assert!(storage.load_state().unwrap().is_none());

        let cluster = run_cluster(&[ClusterEvent::ServerOp {
            ts: 1,
            op: ScriptOp::AddPlaylist {
                file: 1,
                after: None,
            },
        }]);
        let snapshot = StateSnapshot {
            epoch: Epoch(7),
            state: cluster.server,
        };
        storage.save_state(&snapshot, 1000).unwrap();
        assert_eq!(storage.load_state().unwrap().unwrap(), snapshot);
    }

    #[test]
    fn chat_archive_is_idempotent_and_ordered() {
        let mut storage = ServerStorage::open_in_memory().unwrap();
        let messages = vec![msg(2, "b", "second"), msg(1, "a", "first")];
        assert_eq!(storage.archive_chat(&messages).unwrap(), 2);
        // Re-archiving (the next compaction sees overlapping history).
        assert_eq!(storage.archive_chat(&messages).unwrap(), 0);
        storage.archive_chat(&[msg(3, "a", "third")]).unwrap();

        let all = storage.chat_archive(None, 100).unwrap();
        assert_eq!(
            all.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
        let since = storage.chat_archive(Some(2), 100).unwrap();
        assert_eq!(since.len(), 2);
        assert_eq!(storage.chat_archive(None, 1).unwrap().len(), 1);
    }

    #[test]
    fn anidb_queue_lifecycle() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let info = FileHashInfo {
            hash: hash(1),
            size: 1234,
            filename: "ep1.mkv".into(),
        };
        storage.enqueue_lookup(&info, 100).unwrap();
        // Duplicate enqueue (clients re-insert after reconnect): ignored,
        // scheduling state preserved.
        storage.enqueue_lookup(&info, 999).unwrap();

        let due = storage.due_lookups(100, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].info, info);
        assert_eq!(due[0].attempts, 0);
        assert_eq!(due[0].last_attempt, None);
        assert_eq!(due[0].next_attempt, 100);

        // Not due yet.
        assert!(storage.due_lookups(99, 10).unwrap().is_empty());

        // Failed attempt: retry in 5s, still no data.
        storage
            .record_lookup_attempt(hash(1), 100, 105, false)
            .unwrap();
        assert!(storage.due_lookups(104, 10).unwrap().is_empty());
        let due = storage.due_lookups(105, 10).unwrap();
        assert_eq!(due[0].attempts, 1);
        assert_eq!(due[0].last_attempt, Some(100));
        assert!(!due[0].has_data);

        // Success: re-validate much later; has_data sticks.
        storage
            .record_lookup_attempt(hash(1), 105, 700_000, true)
            .unwrap();
        let due = storage.due_lookups(700_000, 10).unwrap();
        assert!(due[0].has_data);

        storage.remove_lookup(hash(1)).unwrap();
        assert!(storage.due_lookups(i64::MAX, 10).unwrap().is_empty());
    }
}
