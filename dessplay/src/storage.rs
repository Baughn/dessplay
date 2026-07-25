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
use std::time::Duration;

use dessplay_core::hash::Ed2kFileHash;
use dessplay_core::types::{AniDbSeriesId, Ed2kHash, Epoch};
use dessplay_core::wire::WireError;
use dessplay_core::{CrdtState, StateSnapshot};
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::Settings;

/// How long a contended writer waits for the write lock before giving up
/// with SQLITE_BUSY. Generous: writes are small and infrequent, and the
/// several same-process connections rarely collide.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

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
    // v2 (Phase 9): the hash cache. A file's ed2k root + per-block
    // hashes, keyed by path and validated by (mtime, size) — so
    // unwatched playlist entries aren't re-hashed every session, and a
    // touched file is re-hashed exactly once (design.md, Content Hash).
    "
    CREATE TABLE hash_cache (
        path        TEXT PRIMARY KEY,
        mtime       INTEGER NOT NULL,  -- unix millis of the file's mtime
        size_bytes  INTEGER NOT NULL,
        root        BLOB NOT NULL,     -- 16-byte ed2k root
        blocks      BLOB NOT NULL,     -- concatenated 16-byte block MD4s
        hashed_at   INTEGER NOT NULL
    ) STRICT;
    ",
    // v3 (2026-06-28): store path columns as a BLOB of the OS-native bytes
    // instead of TEXT, so non-UTF-8 paths (legal on Linux) round-trip
    // losslessly instead of being mangled by `to_string_lossy()`. The
    // affected columns are media_roots.path, cache_entries.path,
    // hash_cache.path, manual_mappings.local_path, series_map_dirs.dir.
    // Existing rows hold valid UTF-8 paths; `CAST(path AS BLOB)` yields
    // their UTF-8 bytes — exactly what `OsStr::as_bytes()` produces for a
    // UTF-8 path — so the new reader reconstructs the same PathBuf. Tables
    // are rebuilt because SQLite cannot change a column's declared type in
    // place, and STRICT rejects a BLOB in a TEXT column.
    "
    CREATE TABLE media_roots_v3 (
        position INTEGER PRIMARY KEY,
        path     BLOB NOT NULL UNIQUE
    ) STRICT;
    INSERT INTO media_roots_v3 (position, path)
        SELECT position, CAST(path AS BLOB) FROM media_roots;
    DROP TABLE media_roots;
    ALTER TABLE media_roots_v3 RENAME TO media_roots;

    CREATE TABLE cache_entries_v3 (
        hash        BLOB PRIMARY KEY,
        path        BLOB NOT NULL,
        size_bytes  INTEGER NOT NULL,
        last_access INTEGER NOT NULL
    ) STRICT;
    INSERT INTO cache_entries_v3 (hash, path, size_bytes, last_access)
        SELECT hash, CAST(path AS BLOB), size_bytes, last_access FROM cache_entries;
    DROP TABLE cache_entries;
    ALTER TABLE cache_entries_v3 RENAME TO cache_entries;

    CREATE TABLE hash_cache_v3 (
        path        BLOB PRIMARY KEY,
        mtime       INTEGER NOT NULL,
        size_bytes  INTEGER NOT NULL,
        root        BLOB NOT NULL,
        blocks      BLOB NOT NULL,
        hashed_at   INTEGER NOT NULL
    ) STRICT;
    INSERT INTO hash_cache_v3 (path, mtime, size_bytes, root, blocks, hashed_at)
        SELECT CAST(path AS BLOB), mtime, size_bytes, root, blocks, hashed_at FROM hash_cache;
    DROP TABLE hash_cache;
    ALTER TABLE hash_cache_v3 RENAME TO hash_cache;

    CREATE TABLE manual_mappings_v3 (
        hash       BLOB PRIMARY KEY,
        local_path BLOB NOT NULL,
        mapped_at  INTEGER NOT NULL
    ) STRICT;
    INSERT INTO manual_mappings_v3 (hash, local_path, mapped_at)
        SELECT hash, CAST(local_path AS BLOB), mapped_at FROM manual_mappings;
    DROP TABLE manual_mappings;
    ALTER TABLE manual_mappings_v3 RENAME TO manual_mappings;

    CREATE TABLE series_map_dirs_v3 (
        series_key TEXT PRIMARY KEY,
        dir        BLOB NOT NULL,
        used_at    INTEGER NOT NULL
    ) STRICT;
    INSERT INTO series_map_dirs_v3 (series_key, dir, used_at)
        SELECT series_key, CAST(dir AS BLOB), used_at FROM series_map_dirs;
    DROP TABLE series_map_dirs;
    ALTER TABLE series_map_dirs_v3 RENAME TO series_map_dirs;
    ",
    // v4 (torrent-first downloads): the ed2k↔infohash mapping for
    // torrents in the engine. The engine's own persistence re-adds
    // torrents by infohash at startup; this table ties them back to the
    // playlist files they fetch/seed, so eviction and reconciliation can
    // remove the right torrent.
    "
    CREATE TABLE torrents (
        hash      BLOB PRIMARY KEY,     -- 16-byte ed2k root
        info_hash TEXT NOT NULL,        -- lowercase hex
        name      TEXT NOT NULL,        -- release title, for logs
        added_at  INTEGER NOT NULL
    ) STRICT;
    ",
    // v5: media-root lifecycle.  Hash rows owned by a library root survive
    // wholesale root disappearance (removable/ZFS storage) while the root
    // record carries whether it is temporarily vanished or was removed from
    // the effective configuration.  Rows outside media roots keep NULL.
    "
    ALTER TABLE hash_cache ADD COLUMN media_root BLOB;
    CREATE TABLE library_roots (
        path        BLOB PRIMARY KEY,
        vanished_at INTEGER,
        removed_at  INTEGER
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

/// Identifies a series for "known series" checks, map-dir memory, and
/// Recent-Series recency: by AniDB id when metadata exists, by parsed
/// name before it does.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// One torrents-table row: the ed2k↔infohash mapping for a torrent the
/// engine holds (torrent-first downloads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorrentRow {
    /// The playlist file the torrent fetches/seeds.
    pub hash: Ed2kHash,
    /// BitTorrent info hash, lowercase hex.
    pub info_hash: String,
    /// Release title (logs / magnet display name).
    pub name: String,
    /// Unix millis when the torrent was added.
    pub added_at: i64,
}

/// One hash-cache row: a path's full ed2k hash, valid while the file's
/// (mtime, size) both still match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedHash {
    /// Absolute path that was hashed.
    pub path: PathBuf,
    /// File mtime at hash time, unix millis.
    pub mtime: i64,
    /// Root + per-block hashes + size.
    pub hash: dessplay_core::hash::Ed2kFileHash,
    /// Media root that owns this library row; `None` for cache/manual rows.
    pub media_root: Option<PathBuf>,
}

/// Durable state for one media root previously seen by the file actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryRoot {
    /// Root path.
    pub path: PathBuf,
    /// When every recorded file first appeared absent.
    pub vanished_at: Option<i64>,
    /// When the root left the effective runtime configuration.
    pub removed_at: Option<i64>,
}

fn hash_from_blob(blob: Vec<u8>) -> Result<Ed2kHash> {
    let bytes: [u8; 16] = blob
        .try_into()
        .map_err(|blob: Vec<u8>| StorageError::Corrupt(format!("hash blob len {}", blob.len())))?;
    Ok(Ed2kHash(bytes))
}

/// Encode a path as the OS-native bytes stored in a BLOB column. On Unix a
/// path is an arbitrary byte sequence, so `to_string_lossy()` would mangle
/// non-UTF-8 paths (substituting U+FFFD) and desync the stored path from
/// disk; storing the raw bytes round-trips losslessly (see the v3
/// migration). On non-Unix (no `OsStrExt`) it falls back to the UTF-8
/// representation — this project targets Linux/NixOS.
#[cfg(unix)]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// Decode a path BLOB written by [`path_to_bytes`].
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
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
        // run_interactive opens several independent write connections to the
        // same file (sync actor, file actor, session, settings) on different
        // tokio tasks. WAL removes reader/writer contention but not
        // writer/writer: with the default busy_timeout of 0 a second
        // concurrent write transaction returns SQLITE_BUSY immediately, and
        // callers log-and-drop the write (a lost hash-cache / watch-history /
        // snapshot row). A timeout makes a contended writer wait and retry.
        conn.busy_timeout(BUSY_TIMEOUT)?;
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
        let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
        let mut roots = Vec::new();
        for row in rows {
            roots.push(path_from_bytes(&row?));
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
                params![position as i64, path_to_bytes(root)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- CRDT snapshot.

    /// Persist the latest full-state snapshot (single implicit room).
    /// Stored in the tagged snapshot envelope (magic + version), not the
    /// raw wire shape — see [`CrdtState::encode_snapshot`].
    pub fn save_state(&self, snapshot: &StateSnapshot, now: i64) -> Result<()> {
        let blob = snapshot.state.encode_snapshot()?;
        let bytes = blob.len();
        self.conn.execute(
            "INSERT INTO crdt_state (room, epoch, state, saved_at)
             VALUES ('default', ?1, ?2, ?3)
             ON CONFLICT (room) DO UPDATE
             SET epoch = excluded.epoch, state = excluded.state,
                 saved_at = excluded.saved_at",
            params![snapshot.epoch.0 as i64, blob, now],
        )?;
        // No timing field here: storage never reads the clock (module doc;
        // design.md, Schema) -- `Instant::now()` is a clock read, and the
        // invariant keeps the layer fully deterministic for tests.
        tracing::debug!(epoch = snapshot.epoch.0, bytes, "state snapshot saved");
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
            tracing::debug!("no stored state snapshot");
            return Ok(None);
        };
        let state = CrdtState::decode_snapshot(&blob)?;
        // No timing field here either -- see save_state.
        tracing::debug!(epoch, bytes = blob.len(), "state snapshot loaded");
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

    /// Every personally-watched file hash. The file browser greys these
    /// out (alongside the group's watched flags).
    pub fn watched_hashes(&self) -> Result<std::collections::BTreeSet<Ed2kHash>> {
        let mut stmt = self.conn.prepare("SELECT hash FROM watch_history")?;
        let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
        let mut hashes = std::collections::BTreeSet::new();
        for row in rows {
            hashes.insert(hash_from_blob(row?)?);
        }
        Ok(hashes)
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
                path_to_bytes(&entry.path),
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
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (blob, path, size_bytes, last_access) = row?;
            entries.push(CacheEntry {
                hash: hash_from_blob(blob)?,
                path: path_from_bytes(&path),
                size_bytes: size_bytes as u64,
                last_access,
            });
        }
        Ok(entries)
    }

    // ---- Torrent registry (torrent-first downloads).

    /// Record (or refresh) the torrent fetching/seeding `hash`.
    pub fn upsert_torrent(&self, row: &TorrentRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO torrents (hash, info_hash, name, added_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (hash) DO UPDATE
             SET info_hash = excluded.info_hash, name = excluded.name,
                 added_at = excluded.added_at",
            params![row.hash.0.as_slice(), row.info_hash, row.name, row.added_at],
        )?;
        Ok(())
    }

    /// Forget the torrent for `hash` (after removing it from the engine).
    pub fn remove_torrent(&self, hash: Ed2kHash) -> Result<()> {
        self.conn.execute(
            "DELETE FROM torrents WHERE hash = ?1",
            params![hash.0.as_slice()],
        )?;
        Ok(())
    }

    /// All registered torrents.
    pub fn torrents(&self) -> Result<Vec<TorrentRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT hash, info_hash, name, added_at FROM torrents ORDER BY added_at")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut torrents = Vec::new();
        for row in rows {
            let (blob, info_hash, name, added_at) = row?;
            torrents.push(TorrentRow {
                hash: hash_from_blob(blob)?,
                info_hash,
                name,
                added_at,
            });
        }
        Ok(torrents)
    }

    // ---- Hash cache.

    /// Insert or refresh a hash-cache row.
    pub fn upsert_hash_cache(
        &self,
        path: &Path,
        mtime: i64,
        hash: &Ed2kFileHash,
        now: i64,
    ) -> Result<()> {
        let blocks: Vec<u8> = hash.blocks.iter().flat_map(|b| b.0).collect();
        self.conn.execute(
            "INSERT INTO hash_cache (path, mtime, size_bytes, root, blocks, hashed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (path) DO UPDATE
             SET mtime = excluded.mtime, size_bytes = excluded.size_bytes,
                 root = excluded.root, blocks = excluded.blocks,
                 hashed_at = excluded.hashed_at",
            params![
                path_to_bytes(path),
                mtime,
                hash.size_bytes as i64,
                hash.root.0.as_slice(),
                blocks,
                now
            ],
        )?;
        Ok(())
    }

    /// Associate an existing hash row with the media root that owns it.
    pub fn set_hash_cache_root(&self, path: &Path, root: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE hash_cache SET media_root = ?2 WHERE path = ?1",
            params![path_to_bytes(path), path_to_bytes(root)],
        )?;
        Ok(())
    }

    /// Every hash-cache row (loaded into memory at session start; the
    /// table is one row per known media file).
    pub fn hash_cache(&self) -> Result<Vec<CachedHash>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, mtime, size_bytes, root, blocks, media_root FROM hash_cache")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, Option<Vec<u8>>>(5)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (path_bytes, mtime, size_bytes, root, blocks, media_root) = row?;
            let path = path_from_bytes(&path_bytes);
            if !blocks.len().is_multiple_of(16) {
                return Err(StorageError::Corrupt(format!(
                    "hash_cache blocks blob len {} for {}",
                    blocks.len(),
                    path.display()
                )));
            }
            entries.push(CachedHash {
                path,
                mtime,
                hash: Ed2kFileHash {
                    root: hash_from_blob(root)?,
                    blocks: blocks
                        .chunks_exact(16)
                        .map(|chunk| {
                            let mut bytes = [0u8; 16];
                            bytes.copy_from_slice(chunk);
                            dessplay_core::hash::Ed2kBlockHash(bytes)
                        })
                        .collect(),
                    size_bytes: size_bytes as u64,
                },
                media_root: media_root.as_deref().map(path_from_bytes),
            });
        }
        Ok(entries)
    }

    /// Every indexed file as (path, ed2k root, mtime millis) — the lean
    /// projection of [`Self::hash_cache`] (no per-block blobs), sized for
    /// the file browser's recursive search on every browser open. mtime
    /// backs the browser's newest-first sort (design.md #8).
    pub fn library_paths(&self) -> Result<Vec<(PathBuf, Ed2kHash, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT h.path, h.root, h.mtime
                 FROM hash_cache h
                 LEFT JOIN library_roots r ON r.path = h.media_root
                 WHERE h.media_root IS NULL
                    OR (r.removed_at IS NULL AND r.vanished_at IS NULL)",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (path_bytes, root, mtime) = row?;
            entries.push((path_from_bytes(&path_bytes), hash_from_blob(root)?, mtime));
        }
        Ok(entries)
    }

    /// Drop a hash-cache row (the file moved or vanished).
    pub fn remove_hash_cache(&self, path: &Path) -> Result<()> {
        self.conn.execute(
            "DELETE FROM hash_cache WHERE path = ?1",
            params![path_to_bytes(path)],
        )?;
        Ok(())
    }

    /// Load every durable media-root lifecycle row.
    pub fn library_roots(&self) -> Result<Vec<LibraryRoot>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, vanished_at, removed_at FROM library_roots")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        let mut roots = Vec::new();
        for row in rows {
            let (path, vanished_at, removed_at) = row?;
            roots.push(LibraryRoot {
                path: path_from_bytes(&path),
                vanished_at,
                removed_at,
            });
        }
        Ok(roots)
    }

    /// Reconcile the effective runtime root list and purge roots whose
    /// removal grace has elapsed. Returns purged root paths.
    pub fn reconcile_library_roots(
        &mut self,
        active: &[PathBuf],
        now: i64,
        grace_millis: i64,
    ) -> Result<Vec<PathBuf>> {
        let tx = self.conn.transaction()?;
        for root in active {
            tx.execute(
                "INSERT INTO library_roots (path, vanished_at, removed_at)
                 VALUES (?1, NULL, NULL)
                 ON CONFLICT(path) DO UPDATE SET removed_at = NULL",
                params![path_to_bytes(root)],
            )?;
        }
        let active_bytes: Vec<Vec<u8>> = active.iter().map(|p| path_to_bytes(p)).collect();
        {
            let mut stmt = tx.prepare("SELECT path FROM library_roots WHERE removed_at IS NULL")?;
            let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
            for row in rows {
                let path = row?;
                if !active_bytes.contains(&path) {
                    tx.execute(
                        "UPDATE library_roots SET removed_at = ?2 WHERE path = ?1",
                        params![path, now],
                    )?;
                }
            }
        }
        let cutoff = now.saturating_sub(grace_millis);
        let mut purged = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT path FROM library_roots
                 WHERE removed_at IS NOT NULL AND removed_at <= ?1",
            )?;
            let rows = stmt.query_map(params![cutoff], |row| row.get::<_, Vec<u8>>(0))?;
            for row in rows {
                purged.push(row?);
            }
        }
        for root in &purged {
            tx.execute(
                "DELETE FROM hash_cache WHERE media_root = ?1",
                params![root],
            )?;
            tx.execute("DELETE FROM library_roots WHERE path = ?1", params![root])?;
        }
        tx.commit()?;
        Ok(purged.iter().map(|p| path_from_bytes(p)).collect())
    }

    /// Mark or clear wholesale disappearance for an active media root.
    pub fn set_library_root_vanished(&self, root: &Path, vanished_at: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE library_roots SET vanished_at = ?2 WHERE path = ?1",
            params![path_to_bytes(root), vanished_at],
        )?;
        Ok(())
    }

    // ---- Manual file mappings.

    /// Map a playlist entry to a local file the user picked.
    pub fn set_manual_mapping(&self, hash: Ed2kHash, local_path: &Path, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO manual_mappings (hash, local_path, mapped_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (hash) DO UPDATE
             SET local_path = excluded.local_path, mapped_at = excluded.mapped_at",
            params![hash.0.as_slice(), path_to_bytes(local_path), now],
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
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|b| path_from_bytes(&b)))
    }

    /// All manual mappings (loaded once at session start; the session
    /// shell consults them before the matcher).
    pub fn manual_mappings(&self) -> Result<Vec<(Ed2kHash, PathBuf)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT hash, local_path FROM manual_mappings")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut mappings = Vec::new();
        for row in rows {
            let (hash, path) = row?;
            let Ok(hash) = <[u8; 16]>::try_from(hash.as_slice()) else {
                continue;
            };
            mappings.push((Ed2kHash(hash), path_from_bytes(&path)));
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
            params![key.as_db_key(), path_to_bytes(dir), now],
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
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|b| path_from_bytes(&b)))
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

    /// Regression: every connection must carry a non-zero busy_timeout, so a
    /// contended writer waits instead of dropping the write with SQLITE_BUSY.
    /// (A full concurrency reproduction would need a timing-dependent sleep to
    /// order two threads; this guards the config invariant deterministically —
    /// remove the busy_timeout line and it fails.)
    #[test]
    fn connections_have_a_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(&dir.path().join("t.db")).unwrap();
        let millis: i64 = storage
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(millis, BUSY_TIMEOUT.as_millis() as i64);
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
    fn undecodable_snapshot_blob_is_a_codec_error() {
        // A blob the current CrdtState can't decode (e.g. a CRDT schema
        // change between versions) must surface as a Codec error, so the
        // client's tolerant loader can drop it and re-sync rather than
        // brick startup. We simulate it with a garbage blob.
        let storage = Storage::open_in_memory().unwrap();
        storage
            .conn
            .execute(
                "INSERT INTO crdt_state (room, epoch, state, saved_at)
                 VALUES ('default', 1, ?1, 0)",
                [&b"not a valid postcard CrdtState"[..]],
            )
            .unwrap();
        assert!(matches!(storage.load_state(), Err(StorageError::Codec(_))));
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
    fn hash_cache_round_trips_and_replaces() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(storage.hash_cache().unwrap().is_empty());

        let hashed = dessplay_core::hash::ed2k_hash_bytes(b"episode contents");
        storage
            .upsert_hash_cache(Path::new("/anime/ep1.mkv"), 1_000, &hashed, 50)
            .unwrap();
        let rows = storage.hash_cache().unwrap();
        assert_eq!(
            rows,
            vec![CachedHash {
                path: PathBuf::from("/anime/ep1.mkv"),
                mtime: 1_000,
                hash: hashed.clone(),
                media_root: None,
            }]
        );

        // A touched file re-hashes: same path, new mtime replaces.
        let rehashed = dessplay_core::hash::ed2k_hash_bytes(b"new contents");
        storage
            .upsert_hash_cache(Path::new("/anime/ep1.mkv"), 2_000, &rehashed, 60)
            .unwrap();
        let rows = storage.hash_cache().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].mtime, 2_000);
        assert_eq!(rows[0].hash, rehashed);

        storage
            .remove_hash_cache(Path::new("/anime/ep1.mkv"))
            .unwrap();
        assert!(storage.hash_cache().unwrap().is_empty());
    }

    #[test]
    fn media_root_vanish_and_removed_grace_lifecycle() {
        const DAY: i64 = 24 * 60 * 60 * 1_000;
        let mut storage = Storage::open_in_memory().unwrap();
        let root = PathBuf::from("/media/removable");
        let path = root.join("show/ep1.mkv");
        let hashed = dessplay_core::hash::ed2k_hash_bytes(b"episode contents");
        storage
            .upsert_hash_cache(&path, 1_000, &hashed, 50)
            .unwrap();
        storage
            .reconcile_library_roots(std::slice::from_ref(&root), 100, 7 * DAY)
            .unwrap();
        storage.set_hash_cache_root(&path, &root).unwrap();

        storage.set_library_root_vanished(&root, Some(200)).unwrap();
        assert_eq!(storage.hash_cache().unwrap().len(), 1);
        assert!(storage.library_paths().unwrap().is_empty());

        // Removal hides immediately, but re-adding inside the grace period
        // clears the clock and keeps the cached hash.
        storage.reconcile_library_roots(&[], 300, 7 * DAY).unwrap();
        storage
            .reconcile_library_roots(std::slice::from_ref(&root), 6 * DAY, 7 * DAY)
            .unwrap();
        assert_eq!(storage.hash_cache().unwrap().len(), 1);
        assert_eq!(storage.library_roots().unwrap()[0].removed_at, None);

        // A new removal clock expires inclusively at seven days.
        storage
            .reconcile_library_roots(&[], 7 * DAY, 7 * DAY)
            .unwrap();
        let purged = storage
            .reconcile_library_roots(&[], 14 * DAY, 7 * DAY)
            .unwrap();
        assert_eq!(purged, vec![root]);
        assert!(storage.hash_cache().unwrap().is_empty());
        assert!(storage.library_roots().unwrap().is_empty());
    }

    #[test]
    fn library_paths_is_the_lean_hash_cache_projection() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(storage.library_paths().unwrap().is_empty());

        let hashed = dessplay_core::hash::ed2k_hash_bytes(b"episode contents");
        storage
            .upsert_hash_cache(Path::new("/anime/ep1.mkv"), 1_000, &hashed, 50)
            .unwrap();
        assert_eq!(
            storage.library_paths().unwrap(),
            vec![(PathBuf::from("/anime/ep1.mkv"), hashed.root, 1_000)]
        );
    }

    #[test]
    fn watched_hashes_collects_all_history() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(storage.watched_hashes().unwrap().is_empty());
        for n in [1u8, 2] {
            storage
                .record_watched(&WatchRecord {
                    hash: hash(n),
                    series_id: None,
                    series_name: None,
                    filename: format!("ep{n}.mkv"),
                    watched_at: n as i64,
                })
                .unwrap();
        }
        assert_eq!(
            storage.watched_hashes().unwrap(),
            [hash(1), hash(2)].into_iter().collect()
        );
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

    /// Paths containing non-UTF-8 bytes (legal on Linux) must round-trip
    /// through every path column without `to_string_lossy()` corruption
    /// (2026-06-26 review). Regression: the columns were TEXT and writes
    /// went through `to_string_lossy()`, so a stored path desynced from
    /// disk and two distinct paths could collide on the lossy form.
    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_round_trip_losslessly() {
        use dessplay_core::hash::{Ed2kBlockHash, Ed2kFileHash};
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        // 0xff/0xfe are not valid UTF-8; this PathBuf has no `to_str()`.
        let bad = PathBuf::from(OsStr::from_bytes(b"/media/\xff\xfeanime/ep.mkv"));
        assert!(bad.to_str().is_none(), "test path must be non-UTF-8");

        let mut storage = Storage::open_in_memory().unwrap();

        // media_roots.path
        storage.set_media_roots(std::slice::from_ref(&bad)).unwrap();
        assert_eq!(storage.media_roots().unwrap(), vec![bad.clone()]);

        // cache_entries.path
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hash(1),
                path: bad.clone(),
                size_bytes: 10,
                last_access: 5,
            })
            .unwrap();
        assert_eq!(storage.cache_entries().unwrap()[0].path, bad);

        // hash_cache.path (incl. the DELETE key matching the same path)
        let fh = Ed2kFileHash {
            root: hash(2),
            blocks: vec![Ed2kBlockHash([7u8; 16])],
            size_bytes: 10,
        };
        storage.upsert_hash_cache(&bad, 100, &fh, 200).unwrap();
        let cached = storage.hash_cache().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].path, bad);
        storage.remove_hash_cache(&bad).unwrap();
        assert!(storage.hash_cache().unwrap().is_empty());

        // manual_mappings.local_path (single + bulk read)
        storage.set_manual_mapping(hash(3), &bad, 1).unwrap();
        assert_eq!(storage.manual_mapping(hash(3)).unwrap(), Some(bad.clone()));
        assert_eq!(
            storage.manual_mappings().unwrap(),
            vec![(hash(3), bad.clone())]
        );

        // series_map_dirs.dir
        let key = SeriesKey::AniDb(AniDbSeriesId(9));
        storage.set_series_map_dir(&key, &bad, 1).unwrap();
        assert_eq!(storage.series_map_dir(&key).unwrap(), Some(bad.clone()));

        // Two distinct non-UTF-8 paths that collapse to the SAME lossy
        // string must both persist — previously this violated the
        // media_roots UNIQUE(path) constraint and failed the whole txn.
        let a = PathBuf::from(OsStr::from_bytes(b"/x/\xff"));
        let b = PathBuf::from(OsStr::from_bytes(b"/x/\xfe"));
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        storage.set_media_roots(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(storage.media_roots().unwrap(), vec![a, b]);
    }

    /// The v3 TEXT->BLOB path migration must carry existing (UTF-8) rows
    /// across unchanged: `CAST(path AS BLOB)` yields the UTF-8 bytes the
    /// new reader expects. Guards the append-only data copy.
    #[test]
    fn v3_migration_preserves_existing_utf8_paths() {
        let conn = Connection::open_in_memory().unwrap();
        // Apply through v2 (the pre-BLOB schema) and insert UTF-8 rows the
        // way the old code did (TEXT paths).
        migrate(&conn, &MIGRATIONS[..2]).unwrap();
        conn.execute(
            "INSERT INTO media_roots (position, path) VALUES (0, '/anime/frieren')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hash_cache (path, mtime, size_bytes, root, blocks, hashed_at)
             VALUES ('/anime/ep1.mkv', 1, 2, ?1, ?2, 3)",
            params![[9u8; 16].as_slice(), [7u8; 16].as_slice()],
        )
        .unwrap();

        // Upgrade to v3 and read back through the BLOB-aware API.
        migrate(&conn, MIGRATIONS).unwrap();
        let storage = Storage { conn };
        assert_eq!(
            storage.media_roots().unwrap(),
            vec![PathBuf::from("/anime/frieren")]
        );
        let cached = storage.hash_cache().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].path, PathBuf::from("/anime/ep1.mkv"));
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
