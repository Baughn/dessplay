//! Client-side SQLite persistence.
//!
//! One database at `$XDG_DATA_HOME/dessplay/dessplay.db` holds everything
//! the client persists: settings, media roots, the latest CRDT snapshot,
//! personal watch history, download-cache bookkeeping, manual file
//! mappings, and TOFU certificate fingerprints.
//!
//! Design notes (see docs/design.md, Data Storage):
//! - CRDT persistence is **snapshot-only** — there is no op log, and
//!   unsent ops are deliberately memory-only.
//! - All timestamps in this module are caller-supplied unix milliseconds
//!   (`i64`); storage never reads the clock, which keeps tests
//!   deterministic.
//! - `Storage` wraps a single `rusqlite::Connection` and is not `Sync`;
//!   the owning actor is the serialization point.

use std::fmt;
use std::path::{Path, PathBuf};

use dessplay_core::types::{AniDbSeriesId, Ed2kHash, Epoch};
use dessplay_core::wire::WireError;
use dessplay_core::{CrdtState, StateSnapshot, wire};
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::Settings;

/// Storage errors. SQLite failures, snapshot (de)serialization failures,
/// or corrupt rows.
#[derive(Debug)]
pub enum StorageError {
    /// Underlying SQLite error.
    Sqlite(rusqlite::Error),
    /// Postcard encode/decode failure on a snapshot blob.
    Codec(WireError),
    /// A stored value failed validation (wrong blob length, bad enum tag).
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

/// The versioned schema. Each entry is one migration; `PRAGMA
/// user_version` records how many have been applied. Append-only: never
/// edit an existing entry once released.
const MIGRATIONS: &[&str] = &[
    // v1: initial schema.
    "
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;

    CREATE TABLE media_roots (
        position INTEGER PRIMARY KEY,  -- 0 = download target
        path     TEXT NOT NULL UNIQUE
    ) STRICT;

    CREATE TABLE crdt_state (
        room     TEXT PRIMARY KEY,     -- single implicit room in v1
        epoch    INTEGER NOT NULL,
        state    BLOB NOT NULL,        -- postcard CrdtState
        saved_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE watch_history (
        hash        BLOB PRIMARY KEY,  -- 16-byte ed2k root
        series_id   INTEGER,           -- AniDB id, if known at watch time
        series_name TEXT,              -- parsed name fallback
        filename    TEXT NOT NULL,
        watched_at  INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX watch_history_series_id ON watch_history (series_id);
    CREATE INDEX watch_history_series_name ON watch_history (series_name);

    CREATE TABLE cache_entries (
        hash        BLOB PRIMARY KEY,
        path        TEXT NOT NULL,
        size_bytes  INTEGER NOT NULL,
        last_access INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE manual_mappings (
        hash       BLOB PRIMARY KEY,   -- playlist entry being mapped
        local_path TEXT NOT NULL,
        mapped_at  INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE series_map_dirs (
        series_key TEXT PRIMARY KEY,   -- anidb:<id> or name:<parsed name>
        dir        TEXT NOT NULL,
        used_at    INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE tofu_fingerprints (
        server      TEXT PRIMARY KEY,  -- host:port
        fingerprint BLOB NOT NULL,     -- SHA-256 of the server cert
        first_seen  INTEGER NOT NULL
    ) STRICT;
    ",
];

/// Apply any unapplied migrations. Exposed shape (a slice parameter) so
/// tests can drive incremental upgrades.
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
        // execute_batch runs inside an implicit transaction per statement;
        // wrap the whole migration + version bump in one explicit one.
        conn.execute_batch(&format!(
            "BEGIN;\n{migration}\nPRAGMA user_version = {};\nCOMMIT;",
            index + 1
        ))?;
        tracing::debug!(version = index + 1, "applied schema migration");
    }
    Ok(())
}

/// Identifies a series for "known series" checks and map-dir memory:
/// by AniDB id when metadata exists, by parsed name before it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeriesKey {
    /// An AniDB-linked series.
    AniDb(AniDbSeriesId),
    /// A series known only by its filename-parsed name.
    Name(String),
}

impl SeriesKey {
    fn as_db_key(&self) -> String {
        match self {
            SeriesKey::AniDb(id) => format!("anidb:{}", id.0),
            SeriesKey::Name(name) => format!("name:{name}"),
        }
    }
}

/// One personal watch-history record. Survives cache eviction: keyed by
/// content hash, with series identity denormalized in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchRecord {
    /// The watched file.
    pub hash: Ed2kHash,
    /// AniDB series id, if metadata was available when watched.
    pub series_id: Option<AniDbSeriesId>,
    /// Parsed series name fallback.
    pub series_name: Option<String>,
    /// Filename at watch time, for display.
    pub filename: String,
    /// Unix millis when the 85% threshold was crossed.
    pub watched_at: i64,
}

/// One download-cache bookkeeping row. The file itself lives in
/// `$XDG_CACHE_HOME/dessplay/files/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    /// The cached file.
    pub hash: Ed2kHash,
    /// Absolute path of the cached file.
    pub path: PathBuf,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Unix millis of last access; drives retention-based eviction.
    pub last_access: i64,
}

fn hash_from_blob(blob: Vec<u8>) -> Result<Ed2kHash> {
    let bytes: [u8; 16] = blob
        .try_into()
        .map_err(|blob: Vec<u8>| StorageError::Corrupt(format!("hash blob len {}", blob.len())))?;
    Ok(Ed2kHash(bytes))
}

/// The client's persistent storage.
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Open (creating and migrating as needed) the database at `path`.
    /// Parent directories are created.
    pub fn open(path: &Path) -> Result<Self> {
        tracing::debug!(path = %path.display(), "opening client database");
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

    /// The default database path: `$XDG_DATA_HOME/dessplay/dessplay.db`.
    pub fn default_path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("dessplay").join("dessplay.db"))
    }

    // ---- Settings (see config.rs for the typed struct).

    /// Read one settings key.
    pub(crate) fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Write (or clear) one settings key.
    pub(crate) fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => {
                self.conn.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    params![key, value],
                )?;
            }
            None => {
                self.conn
                    .execute("DELETE FROM settings WHERE key = ?1", params![key])?;
            }
        }
        Ok(())
    }

    /// Load typed settings (defaults fill any missing keys).
    pub fn load_settings(&self) -> Result<Settings> {
        Settings::load(self)
    }

    /// Persist typed settings.
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        settings.save(self)
    }

    /// Media roots in priority order; index 0 is the download target.
    pub fn media_roots(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM media_roots ORDER BY position")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut roots = Vec::new();
        for row in rows {
            roots.push(PathBuf::from(row?));
        }
        Ok(roots)
    }

    /// Replace the media root list (ordering is the slice order).
    pub fn set_media_roots(&mut self, roots: &[PathBuf]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM media_roots", [])?;
        for (position, root) in roots.iter().enumerate() {
            tx.execute(
                "INSERT INTO media_roots (position, path) VALUES (?1, ?2)",
                params![position as i64, root.to_string_lossy()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- CRDT snapshot.

    /// Persist the latest full-state snapshot (single implicit room).
    pub fn save_state(&self, snapshot: &StateSnapshot, now: i64) -> Result<()> {
        let started = std::time::Instant::now();
        let blob = wire::encode(&snapshot.state)?;
        let bytes = blob.len();
        self.conn.execute(
            "INSERT INTO crdt_state (room, epoch, state, saved_at)
             VALUES ('default', ?1, ?2, ?3)
             ON CONFLICT (room) DO UPDATE
             SET epoch = excluded.epoch, state = excluded.state,
                 saved_at = excluded.saved_at",
            params![snapshot.epoch.0 as i64, blob, now],
        )?;
        tracing::debug!(
            epoch = snapshot.epoch.0,
            bytes,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "state snapshot saved"
        );
        Ok(())
    }

    /// Load the stored snapshot, if any.
    pub fn load_state(&self) -> Result<Option<StateSnapshot>> {
        let started = std::time::Instant::now();
        let row: Option<(i64, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT epoch, state FROM crdt_state WHERE room = 'default'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((epoch, blob)) = row else {
            tracing::debug!("no stored state snapshot");
            return Ok(None);
        };
        let state: CrdtState = wire::decode(&blob)?;
        tracing::debug!(
            epoch,
            bytes = blob.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "state snapshot loaded"
        );
        Ok(Some(StateSnapshot {
            epoch: Epoch(epoch as u64),
            state,
        }))
    }

    // ---- Watch history.

    /// Record (or refresh) a watched file. Last write wins per hash.
    pub fn record_watched(&self, record: &WatchRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO watch_history (hash, series_id, series_name, filename, watched_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (hash) DO UPDATE
             SET series_id = excluded.series_id, series_name = excluded.series_name,
                 filename = excluded.filename, watched_at = excluded.watched_at",
            params![
                record.hash.0.as_slice(),
                record.series_id.map(|id| id.0 as i64),
                record.series_name,
                record.filename,
                record.watched_at
            ],
        )?;
        Ok(())
    }

    /// Look up a watch record by file hash.
    pub fn watched(&self, hash: Ed2kHash) -> Result<Option<WatchRecord>> {
        self.conn
            .query_row(
                "SELECT hash, series_id, series_name, filename, watched_at
                 FROM watch_history WHERE hash = ?1",
                params![hash.0.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?
            .map(|(blob, series_id, series_name, filename, watched_at)| {
                Ok(WatchRecord {
                    hash: hash_from_blob(blob)?,
                    series_id: series_id.map(|id| AniDbSeriesId(id as u32)),
                    series_name,
                    filename,
                    watched_at,
                })
            })
            .transpose()
    }

    /// "Known series": has any file from this series ever been watched?
    pub fn series_known(&self, key: &SeriesKey) -> Result<bool> {
        let count: i64 = match key {
            SeriesKey::AniDb(id) => self.conn.query_row(
                "SELECT COUNT(*) FROM watch_history WHERE series_id = ?1",
                params![id.0 as i64],
                |row| row.get(0),
            )?,
            SeriesKey::Name(name) => self.conn.query_row(
                "SELECT COUNT(*) FROM watch_history WHERE series_name = ?1",
                params![name],
                |row| row.get(0),
            )?,
        };
        Ok(count > 0)
    }

    /// The most recent watch records, newest first. Recent Series
    /// sorting (Phase 9) builds on this.
    pub fn recent_watched(&self, limit: usize) -> Result<Vec<WatchRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, series_id, series_name, filename, watched_at
             FROM watch_history ORDER BY watched_at DESC, hash LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (blob, series_id, series_name, filename, watched_at) = row?;
            records.push(WatchRecord {
                hash: hash_from_blob(blob)?,
                series_id: series_id.map(|id| AniDbSeriesId(id as u32)),
                series_name,
                filename,
                watched_at,
            });
        }
        Ok(records)
    }

    // ---- Download cache bookkeeping.

    /// Insert or update a cache entry.
    pub fn upsert_cache_entry(&self, entry: &CacheEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO cache_entries (hash, path, size_bytes, last_access)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (hash) DO UPDATE
             SET path = excluded.path, size_bytes = excluded.size_bytes,
                 last_access = excluded.last_access",
            params![
                entry.hash.0.as_slice(),
                entry.path.to_string_lossy(),
                entry.size_bytes as i64,
                entry.last_access
            ],
        )?;
        Ok(())
    }

    /// Bump an entry's last-access time. No-op if absent.
    pub fn touch_cache_entry(&self, hash: Ed2kHash, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE cache_entries SET last_access = ?2 WHERE hash = ?1",
            params![hash.0.as_slice(), now],
        )?;
        Ok(())
    }

    /// Forget a cache entry (after deleting the file).
    pub fn remove_cache_entry(&self, hash: Ed2kHash) -> Result<()> {
        self.conn.execute(
            "DELETE FROM cache_entries WHERE hash = ?1",
            params![hash.0.as_slice()],
        )?;
        Ok(())
    }

    /// All cache entries, least recently accessed first.
    pub fn cache_entries(&self) -> Result<Vec<CacheEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, path, size_bytes, last_access
             FROM cache_entries ORDER BY last_access, hash",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (blob, path, size_bytes, last_access) = row?;
            entries.push(CacheEntry {
                hash: hash_from_blob(blob)?,
                path: PathBuf::from(path),
                size_bytes: size_bytes as u64,
                last_access,
            });
        }
        Ok(entries)
    }

    // ---- Manual file mappings.

    /// Map a playlist entry to a local file the user picked.
    pub fn set_manual_mapping(&self, hash: Ed2kHash, local_path: &Path, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO manual_mappings (hash, local_path, mapped_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (hash) DO UPDATE
             SET local_path = excluded.local_path, mapped_at = excluded.mapped_at",
            params![hash.0.as_slice(), local_path.to_string_lossy(), now],
        )?;
        Ok(())
    }

    /// Look up a manual mapping.
    pub fn manual_mapping(&self, hash: Ed2kHash) -> Result<Option<PathBuf>> {
        Ok(self
            .conn
            .query_row(
                "SELECT local_path FROM manual_mappings WHERE hash = ?1",
                params![hash.0.as_slice()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(PathBuf::from))
    }

    /// All manual mappings (loaded once at session start; the session
    /// shell consults them before the matcher).
    pub fn manual_mappings(&self) -> Result<Vec<(Ed2kHash, PathBuf)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT hash, local_path FROM manual_mappings")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut mappings = Vec::new();
        for row in rows {
            let (hash, path) = row?;
            let Ok(hash) = <[u8; 16]>::try_from(hash.as_slice()) else {
                continue;
            };
            mappings.push((Ed2kHash(hash), PathBuf::from(path)));
        }
        Ok(mappings)
    }

    /// Drop a manual mapping.
    pub fn clear_manual_mapping(&self, hash: Ed2kHash) -> Result<()> {
        self.conn.execute(
            "DELETE FROM manual_mappings WHERE hash = ?1",
            params![hash.0.as_slice()],
        )?;
        Ok(())
    }

    /// Remember the directory last used to map a file of this series.
    pub fn set_series_map_dir(&self, key: &SeriesKey, dir: &Path, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO series_map_dirs (series_key, dir, used_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (series_key) DO UPDATE
             SET dir = excluded.dir, used_at = excluded.used_at",
            params![key.as_db_key(), dir.to_string_lossy(), now],
        )?;
        Ok(())
    }

    /// The directory last used to map a file of this series.
    pub fn series_map_dir(&self, key: &SeriesKey) -> Result<Option<PathBuf>> {
        Ok(self
            .conn
            .query_row(
                "SELECT dir FROM series_map_dirs WHERE series_key = ?1",
                params![key.as_db_key()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(PathBuf::from))
    }

    // ---- TOFU certificate fingerprints.

    /// The pinned fingerprint for a server, if we've connected before.
    pub fn tofu_fingerprint(&self, server: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT fingerprint FROM tofu_fingerprints WHERE server = ?1",
                params![server],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Pin a fingerprint on first connection. Refuses to overwrite an
    /// existing pin — replacing a changed cert must be an explicit user
    /// action (delete + re-pin).
    pub fn store_tofu_fingerprint(&self, server: &str, fingerprint: &[u8], now: i64) -> Result<()> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO tofu_fingerprints (server, fingerprint, first_seen)
             VALUES (?1, ?2, ?3)",
            params![server, fingerprint, now],
        )?;
        if changed == 0 {
            return Err(StorageError::Corrupt(format!(
                "refusing to overwrite pinned fingerprint for {server}"
            )));
        }
        Ok(())
    }

    /// Drop a pinned fingerprint (explicit user action after a cert
    /// change).
    pub fn forget_tofu_fingerprint(&self, server: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM tofu_fingerprints WHERE server = ?1",
            params![server],
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

    #[test]
    fn migrations_are_idempotent_and_incremental() {
        let conn = Connection::open_in_memory().unwrap();
        // Apply only v1, then everything: must not error or re-run v1.
        migrate(&conn, &MIGRATIONS[..1]).unwrap();
        migrate(&conn, MIGRATIONS).unwrap();
        migrate(&conn, MIGRATIONS).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());

        // Incremental upgrades apply only the new tail.
        let extra = "CREATE TABLE phase_two_test (x INTEGER);";
        let mut with_extra: Vec<&str> = MIGRATIONS.to_vec();
        with_extra.push(extra);
        migrate(&conn, &with_extra).unwrap();
        conn.execute("INSERT INTO phase_two_test (x) VALUES (1)", [])
            .unwrap();
    }

    #[test]
    fn future_database_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            migrate(&conn, MIGRATIONS),
            Err(StorageError::Corrupt(_))
        ));
    }

    #[test]
    fn snapshot_round_trips_through_db() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(storage.load_state().unwrap().is_none());

        // A nontrivial state via the shared cluster generator.
        let cluster = run_cluster(&[
            ClusterEvent::ClientOp {
                client: 0,
                ts: 1,
                op: ScriptOp::AddPlaylist {
                    file: 1,
                    after: None,
                },
            },
            ClusterEvent::ClientOp {
                client: 1,
                ts: 2,
                op: ScriptOp::Chat { text: 7 },
            },
            ClusterEvent::ServerOp {
                ts: 3,
                op: ScriptOp::SetWatched {
                    file: 1,
                    watched: true,
                },
            },
        ]);
        let snapshot = StateSnapshot {
            epoch: Epoch(42),
            state: cluster.server,
        };

        storage.save_state(&snapshot, 1000).unwrap();
        let loaded = storage.load_state().unwrap().unwrap();
        assert_eq!(loaded, snapshot);
        assert_eq!(loaded.state.view(), snapshot.state.view());

        // Overwrite with a newer epoch.
        let newer = StateSnapshot {
            epoch: Epoch(43),
            state: snapshot.state.clone(),
        };
        storage.save_state(&newer, 2000).unwrap();
        assert_eq!(storage.load_state().unwrap().unwrap().epoch, Epoch(43));
    }

    #[test]
    fn media_roots_keep_order() {
        let mut storage = Storage::open_in_memory().unwrap();
        assert!(storage.media_roots().unwrap().is_empty());
        let roots = vec![
            PathBuf::from("/mnt/nas/anime"),
            PathBuf::from("/home/user/anime"),
        ];
        storage.set_media_roots(&roots).unwrap();
        assert_eq!(storage.media_roots().unwrap(), roots);

        // Reorder: replacement wins, order preserved.
        let reordered = vec![roots[1].clone(), roots[0].clone()];
        storage.set_media_roots(&reordered).unwrap();
        assert_eq!(storage.media_roots().unwrap(), reordered);
    }

    #[test]
    fn watch_history_known_series_and_recency() {
        let storage = Storage::open_in_memory().unwrap();
        let record = WatchRecord {
            hash: hash(1),
            series_id: Some(AniDbSeriesId(5)),
            series_name: Some("Frieren".into()),
            filename: "frieren-01.mkv".into(),
            watched_at: 100,
        };
        storage.record_watched(&record).unwrap();
        storage
            .record_watched(&WatchRecord {
                hash: hash(2),
                series_id: None,
                series_name: Some("GochiUsa".into()),
                filename: "gochiusa-01.mkv".into(),
                watched_at: 200,
            })
            .unwrap();

        assert_eq!(storage.watched(hash(1)).unwrap().unwrap(), record);
        assert!(storage.watched(hash(9)).unwrap().is_none());

        assert!(
            storage
                .series_known(&SeriesKey::AniDb(AniDbSeriesId(5)))
                .unwrap()
        );
        assert!(
            !storage
                .series_known(&SeriesKey::AniDb(AniDbSeriesId(6)))
                .unwrap()
        );
        assert!(
            storage
                .series_known(&SeriesKey::Name("GochiUsa".into()))
                .unwrap()
        );
        assert!(
            !storage
                .series_known(&SeriesKey::Name("Unknown".into()))
                .unwrap()
        );

        let recent = storage.recent_watched(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].hash, hash(2)); // newest first
        assert_eq!(storage.recent_watched(1).unwrap().len(), 1);

        // Re-watching refreshes the timestamp.
        storage
            .record_watched(&WatchRecord {
                watched_at: 300,
                ..record
            })
            .unwrap();
        assert_eq!(storage.recent_watched(1).unwrap()[0].hash, hash(1));
    }

    #[test]
    fn cache_entries_touch_and_evict_order() {
        let storage = Storage::open_in_memory().unwrap();
        for (i, access) in [(1u8, 100i64), (2, 50)] {
            storage
                .upsert_cache_entry(&CacheEntry {
                    hash: hash(i),
                    path: PathBuf::from(format!("/cache/{i}.mkv")),
                    size_bytes: 1000,
                    last_access: access,
                })
                .unwrap();
        }
        // Least recently accessed first.
        let entries = storage.cache_entries().unwrap();
        assert_eq!(entries[0].hash, hash(2));

        storage.touch_cache_entry(hash(2), 500).unwrap();
        assert_eq!(storage.cache_entries().unwrap()[0].hash, hash(1));

        storage.remove_cache_entry(hash(1)).unwrap();
        assert_eq!(storage.cache_entries().unwrap().len(), 1);
    }

    #[test]
    fn manual_mappings_and_series_dirs() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(storage.manual_mapping(hash(1)).unwrap().is_none());

        storage
            .set_manual_mapping(hash(1), Path::new("/anime/alt-name.mkv"), 100)
            .unwrap();
        assert_eq!(
            storage.manual_mapping(hash(1)).unwrap().unwrap(),
            PathBuf::from("/anime/alt-name.mkv")
        );
        storage.clear_manual_mapping(hash(1)).unwrap();
        assert!(storage.manual_mapping(hash(1)).unwrap().is_none());

        let key = SeriesKey::AniDb(AniDbSeriesId(5));
        storage
            .set_series_map_dir(&key, Path::new("/anime/frieren"), 100)
            .unwrap();
        assert_eq!(
            storage.series_map_dir(&key).unwrap().unwrap(),
            PathBuf::from("/anime/frieren")
        );
        // Distinct keyspace for name keys.
        assert!(
            storage
                .series_map_dir(&SeriesKey::Name("frieren".into()))
                .unwrap()
                .is_none()
        );
    }

    /// The Phase 2 milestone: CRDT state and config survive a process
    /// restart (drop the connection, reopen the same file).
    #[test]
    fn state_and_config_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dessplay.db");

        let cluster = run_cluster(&[
            ClusterEvent::ClientOp {
                client: 0,
                ts: 1,
                op: ScriptOp::AddPlaylist {
                    file: 1,
                    after: None,
                },
            },
            ClusterEvent::ClientOp {
                client: 1,
                ts: 2,
                op: ScriptOp::Chat { text: 3 },
            },
        ]);
        let snapshot = StateSnapshot {
            epoch: Epoch(9),
            state: cluster.server,
        };
        let settings = crate::config::Settings {
            username: Some("Baughn".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        };
        let roots = vec![PathBuf::from("/mnt/nas/anime")];

        {
            let mut storage = Storage::open(&db_path).unwrap();
            storage.save_settings(&settings).unwrap();
            storage.set_media_roots(&roots).unwrap();
            storage.save_state(&snapshot, 1000).unwrap();
            storage
                .record_watched(&WatchRecord {
                    hash: hash(1),
                    series_id: None,
                    series_name: Some("Frieren".into()),
                    filename: "frieren-01.mkv".into(),
                    watched_at: 50,
                })
                .unwrap();
        } // drop = process exit

        let storage = Storage::open(&db_path).unwrap();
        assert_eq!(storage.load_settings().unwrap(), settings);
        assert_eq!(storage.media_roots().unwrap(), roots);
        let loaded = storage.load_state().unwrap().unwrap();
        assert_eq!(loaded, snapshot);
        assert_eq!(loaded.state.view(), snapshot.state.view());
        assert!(storage.watched(hash(1)).unwrap().is_some());
    }

    #[test]
    fn tofu_pins_are_write_once() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(
            storage
                .tofu_fingerprint("dessplay.brage.info:443")
                .unwrap()
                .is_none()
        );

        storage
            .store_tofu_fingerprint("dessplay.brage.info:443", &[1, 2, 3], 100)
            .unwrap();
        assert_eq!(
            storage
                .tofu_fingerprint("dessplay.brage.info:443")
                .unwrap()
                .unwrap(),
            vec![1, 2, 3]
        );

        // Overwrite refused.
        assert!(
            storage
                .store_tofu_fingerprint("dessplay.brage.info:443", &[9, 9, 9], 200)
                .is_err()
        );

        // Explicit forget, then re-pin.
        storage
            .forget_tofu_fingerprint("dessplay.brage.info:443")
            .unwrap();
        storage
            .store_tofu_fingerprint("dessplay.brage.info:443", &[9, 9, 9], 200)
            .unwrap();
    }
}
