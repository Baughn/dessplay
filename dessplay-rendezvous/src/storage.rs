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

use std::fmt;
use std::path::{Path, PathBuf};

use dessplay_core::types::{
    AniDbSeriesId, ChatMessage, Ed2kHash, Epoch, FileHashInfo, SharedTimestamp, UserId,
};
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
    // v2 (Phase 8): the ANIME lookup queue (relations walks survive
    // restarts — the graph fills in over hours), the anime-titles dump
    // for name search, and a small kv table for bookkeeping like the
    // dump's last fetch time.
    "
    CREATE TABLE anime_queue (
        aid          INTEGER PRIMARY KEY,
        first_seen   INTEGER NOT NULL,
        last_attempt INTEGER,           -- NULL = never tried
        next_attempt INTEGER NOT NULL,  -- i64::MAX = settled, never retry
        attempts     INTEGER NOT NULL DEFAULT 0
    ) STRICT;
    CREATE INDEX anime_queue_next_attempt ON anime_queue (next_attempt);

    CREATE TABLE anidb_titles (
        aid   INTEGER NOT NULL,
        kind  INTEGER NOT NULL,  -- 1 primary, 2 synonym, 3 short, 4 official
        lang  TEXT NOT NULL,
        title TEXT NOT NULL
    ) STRICT;
    CREATE INDEX anidb_titles_aid ON anidb_titles (aid);

    CREATE TABLE kv (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;
    ",
    // v3: the file's mtime (unix millis), supplied by clients in the
    // lookup request. The NoData re-validation ladder anchors on the
    // *older* of mtime and first_seen, so long-owned files AniDB doesn't
    // know aren't re-polled on the aggressive new-file cadence after a
    // queue reset. NULL = unknown (e.g. a playlist-only request).
    "ALTER TABLE anidb_queue ADD COLUMN mtime INTEGER;",
    // v4: a title-like containing-directory name, supplied by clients that
    // hold the file (e.g. `RahXephon` for `<root>/RahXephon/Season 1/...`).
    // When AniDB doesn't know the file, the fallback series name uses this
    // instead of the per-episode filename stem, so a series' episodes group
    // into one franchise. NULL = unknown (playlist-only request, or no
    // ancestor directory looked like a title).
    "ALTER TABLE anidb_queue ADD COLUMN series_hint TEXT;",
    // v5 (Phase 16, #15): every username ever seen, with a last-seen
    // timestamp, updated on connect/disconnect — survives server restarts
    // (unlike the in-memory peer registry), so a user who hasn't connected
    // yet today can still be named and acted on (`n` / `/skip <name>`).
    "
    CREATE TABLE known_users (
        username  TEXT PRIMARY KEY,
        last_seen INTEGER NOT NULL
    ) STRICT;
    ",
];

/// `next_attempt` sentinel for queue entries that are settled and must
/// never be retried (kept as tombstones so re-discovery is a no-op).
pub const NEVER: i64 = i64::MAX;

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
        tracing::debug!(version = index + 1, "applied schema migration");
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
        tracing::debug!(path = %path.display(), "opening server database");
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
        let state = CrdtState::decode_snapshot(&blob)?;
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

    // ---- Known users (design.md #15).

    /// Record that `username` was seen (connected or disconnected) at
    /// `at`. Upserts — a later call always wins, never regresses.
    pub fn record_seen(&self, username: &UserId, at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO known_users (username, last_seen)
             VALUES (?1, ?2)
             ON CONFLICT (username) DO UPDATE SET last_seen = excluded.last_seen",
            params![username.0, at],
        )?;
        Ok(())
    }

    /// Every known user last seen at or after `cutoff` (shared-clock
    /// millis), for the `PeerList`'s `known_offline` field — the caller
    /// filters out anyone currently in the live registry.
    pub fn known_users(&self, cutoff: i64) -> Result<Vec<(UserId, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT username, last_seen FROM known_users WHERE last_seen >= ?1")?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut users = Vec::new();
        for row in rows {
            let (username, last_seen) = row?;
            users.push((UserId(username), last_seen));
        }
        Ok(users)
    }

    // ---- AniDB validation queue.

    /// Add a lookup request if it isn't already queued. New entries are
    /// due immediately (`next_attempt = now`).
    ///
    /// A request for an already-queued hash does **not** reset the
    /// schedule (`first_seen`, `next_attempt`, `has_data` are left alone),
    /// but it *does* lower the stored `mtime` toward the oldest value
    /// seen, so an existing row learns the file's real age the first time
    /// a client reports it (and never loses a more-aged value). It also
    /// learns a `series_hint` the first time one is reported (keeping the
    /// first non-null hint), so a row queued before the holder reported
    /// — or by a client that didn't hold the file — picks one up later.
    pub fn enqueue_lookup(&self, info: &FileHashInfo, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO anidb_queue
             (hash, size_bytes, filename, first_seen, next_attempt, mtime, series_hint)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6)
             ON CONFLICT(hash) DO UPDATE SET
                 mtime = CASE
                     WHEN excluded.mtime IS NULL THEN mtime
                     WHEN mtime IS NULL THEN excluded.mtime
                     ELSE min(mtime, excluded.mtime)
                 END,
                 series_hint = COALESCE(series_hint, excluded.series_hint)",
            params![
                info.hash.0.as_slice(),
                info.size as i64,
                info.filename,
                now,
                info.mtime,
                info.series_hint,
            ],
        )?;
        Ok(())
    }

    /// Entries due at or before `now`, soonest first.
    pub fn due_lookups(&self, now: i64, limit: usize) -> Result<Vec<QueueEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, size_bytes, filename, first_seen, last_attempt,
                    next_attempt, attempts, has_data, mtime, series_hint
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
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (
                blob,
                size,
                filename,
                first_seen,
                last_attempt,
                next_attempt,
                attempts,
                has_data,
                mtime,
                series_hint,
            ) = row?;
            entries.push(QueueEntry {
                info: FileHashInfo {
                    hash: hash_from_blob(blob)?,
                    size: size as u64,
                    filename,
                    mtime,
                    series_hint,
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

    /// Every file for which a client has reported a title-like directory
    /// hint, as `(hash, series_hint)`. The worker reconciles filename-derived
    /// metadata against these: the fallback series name is written once, but
    /// a hint can be learned afterward (a playlist add carries no hint and
    /// races ahead of the library scan that does), so an early file would
    /// otherwise keep a per-episode stem name and split off into its own
    /// franchise. Independent of the lookup schedule -- a settled file is
    /// reconciled without an AniDB call.
    pub fn series_hints(&self) -> Result<Vec<(Ed2kHash, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT hash, series_hint FROM anidb_queue WHERE series_hint IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (blob, hint) = row?;
            out.push((hash_from_blob(blob)?, hint));
        }
        Ok(out)
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

    /// Re-arm "settled" file lookups (`has_data = 1`, so AniDB knew the
    /// file and the next attempt is a week out) whose hash is **absent**
    /// from `present` — the set of hashes that actually have replicated
    /// metadata. Such a row is a lie: the lookup succeeded and recorded
    /// the attempt durably in SQLite, but the metadata write lived only in
    /// the periodically-snapshotted CRDT state and was lost to a restart
    /// before it persisted. The file is then orphaned — no metadata, and
    /// not re-checked for a week.
    ///
    /// We reset such rows to due now and clear `has_data`, so the next
    /// pass looks them up again (writing metadata on success, or the
    /// filename fallback on a miss). NoData rows are left alone: they
    /// self-heal on their short retry ladder. Returns the re-armed
    /// filenames, for logging.
    pub fn rearm_settled_without_metadata(
        &self,
        present: &std::collections::BTreeSet<Ed2kHash>,
        now: i64,
    ) -> Result<Vec<String>> {
        let orphaned: Vec<(Vec<u8>, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT hash, filename FROM anidb_queue WHERE has_data = 1")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (blob, filename) = row?;
                if !present.contains(&hash_from_blob(blob.clone())?) {
                    out.push((blob, filename));
                }
            }
            out
        };
        let mut rearmed = Vec::with_capacity(orphaned.len());
        for (blob, filename) in orphaned {
            self.conn.execute(
                "UPDATE anidb_queue SET next_attempt = ?2, has_data = 0 WHERE hash = ?1",
                params![blob, now],
            )?;
            rearmed.push(filename);
        }
        Ok(rearmed)
    }

    // ---- ANIME (relations) queue.

    /// Queue a series for an ANIME lookup if it isn't queued already
    /// (settled tombstones included — re-discovery is a no-op).
    pub fn enqueue_anime(&self, series: AniDbSeriesId, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO anime_queue (aid, first_seen, next_attempt)
             VALUES (?1, ?2, ?2)",
            params![series.0 as i64, now],
        )?;
        Ok(())
    }

    /// Series lookups due at or before `now`, soonest first.
    pub fn due_anime(&self, now: i64, limit: usize) -> Result<Vec<AnimeQueueEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT aid, first_seen, last_attempt, next_attempt, attempts
             FROM anime_queue WHERE next_attempt <= ?1
             ORDER BY next_attempt, aid LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now, limit as i64], |row| {
            Ok(AnimeQueueEntry {
                series: AniDbSeriesId(row.get::<_, i64>(0)? as u32),
                first_seen: row.get(1)?,
                last_attempt: row.get(2)?,
                next_attempt: row.get(3)?,
                attempts: row.get::<_, i64>(4)? as u32,
            })
        })?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }

    /// Record an ANIME attempt. `next_attempt = NEVER` settles the
    /// entry (success, or a definitive "no such anime").
    pub fn record_anime_attempt(
        &self,
        series: AniDbSeriesId,
        attempted_at: i64,
        next_attempt: i64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE anime_queue
             SET last_attempt = ?2, next_attempt = ?3, attempts = attempts + 1
             WHERE aid = ?1",
            params![series.0 as i64, attempted_at, next_attempt],
        )?;
        Ok(())
    }

    /// Re-arm "settled" ANIME (relations) lookups whose series id is
    /// **absent** from `present` — the set of series that actually have
    /// replicated relations. This is the relations-graph analogue of
    /// [`Self::rearm_settled_without_metadata`]: a settled row
    /// (`next_attempt = NEVER`) means the ANIME lookup ran and recorded its
    /// attempt durably in SQLite, but on a hit the relations write lived
    /// only in the periodically-snapshotted CRDT state and was lost to a
    /// restart before it persisted. The series is then orphaned —
    /// `enqueue_anime` is `INSERT OR IGNORE` so re-discovery is a no-op
    /// against the tombstone, and `due_anime` never returns it, so its
    /// franchise grouping stays broken forever (it falls back to grouping
    /// by parsed name).
    ///
    /// We reset such rows to due now, so the next pass looks them up again.
    /// A definitive "no such anime" miss also settles with no relations and
    /// is therefore re-armed and re-polled once; that is rare (relation
    /// targets generally exist) and simply re-settles. Returns the re-armed
    /// series ids, for logging.
    pub fn rearm_settled_anime_without_relations(
        &self,
        present: &std::collections::BTreeSet<AniDbSeriesId>,
        now: i64,
    ) -> Result<Vec<AniDbSeriesId>> {
        let orphaned: Vec<i64> = {
            let mut stmt = self
                .conn
                .prepare("SELECT aid FROM anime_queue WHERE next_attempt = ?1")?;
            let rows = stmt.query_map(params![NEVER], |row| row.get::<_, i64>(0))?;
            let mut out = Vec::new();
            for row in rows {
                let aid = row?;
                if !present.contains(&AniDbSeriesId(aid as u32)) {
                    out.push(aid);
                }
            }
            out
        };
        let mut rearmed = Vec::with_capacity(orphaned.len());
        for aid in orphaned {
            self.conn.execute(
                "UPDATE anime_queue SET next_attempt = ?2 WHERE aid = ?1",
                params![aid, now],
            )?;
            rearmed.push(AniDbSeriesId(aid as u32));
        }
        Ok(rearmed)
    }

    /// The earliest scheduled attempt across both lookup queues
    /// (settled [`NEVER`] tombstones excluded). Lets the worker sleep
    /// until something is actually due.
    pub fn next_attempt_at(&self) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT MIN(next) FROM (
                     SELECT MIN(next_attempt) AS next FROM anidb_queue
                      WHERE next_attempt < ?1
                     UNION ALL
                     SELECT MIN(next_attempt) FROM anime_queue
                      WHERE next_attempt < ?1
                 )",
                params![NEVER],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    // ---- Anime-titles dump (name search).

    /// Replace the whole titles table with a fresh dump.
    pub fn replace_titles(&mut self, titles: &[TitleRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM anidb_titles", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO anidb_titles (aid, kind, lang, title) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for row in titles {
                stmt.execute(params![
                    row.series.0 as i64,
                    row.kind as i64,
                    row.lang,
                    row.title
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Case-insensitive (ASCII) substring search over all titles and
    /// synonyms. Ranking: exact match, then prefix, then substring;
    /// shorter titles first within a rank. One hit per series, showing
    /// the matched title alongside the series' primary title.
    pub fn search_titles(&self, query: &str, limit: usize) -> Result<Vec<TitleSearchHit>> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let mut stmt = self.conn.prepare(
            "SELECT aid, title,
                    CASE WHEN title = ?1 COLLATE NOCASE THEN 0
                         WHEN title LIKE ?2 ESCAPE '\\' THEN 1
                         ELSE 2 END AS rank
             FROM anidb_titles
             WHERE title LIKE ?3 ESCAPE '\\'
             ORDER BY rank, length(title), aid
             LIMIT 400",
        )?;
        let rows = stmt.query_map(
            params![query, format!("{escaped}%"), format!("%{escaped}%")],
            |row| {
                Ok((
                    AniDbSeriesId(row.get::<_, i64>(0)? as u32),
                    row.get::<_, String>(1)?,
                ))
            },
        )?;
        let mut seen = std::collections::BTreeSet::new();
        let mut hits = Vec::new();
        for row in rows {
            let (series, matched) = row?;
            if !seen.insert(series) {
                continue;
            }
            let primary = self
                .primary_title(series)?
                .unwrap_or_else(|| matched.clone());
            hits.push(TitleSearchHit {
                series,
                title: primary,
                matched,
            });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }

    /// A series' primary title (kind 1), falling back to any official
    /// title (kind 4).
    fn primary_title(&self, series: AniDbSeriesId) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT title FROM anidb_titles
                 WHERE aid = ?1 AND kind IN (1, 4)
                 ORDER BY kind, lang LIMIT 1",
                params![series.0 as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    // ---- Bookkeeping kv.

    /// Read a bookkeeping value.
    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM kv WHERE key = ?1", params![key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(Into::into)
    }

    /// Write a bookkeeping value.
    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

/// One ANIME (relations) queue row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimeQueueEntry {
    /// The series to look up.
    pub series: AniDbSeriesId,
    /// When the series was first discovered.
    pub first_seen: i64,
    /// Last attempt time; `None` = never tried.
    pub last_attempt: Option<i64>,
    /// Earliest time of the next attempt ([`NEVER`] = settled).
    pub next_attempt: i64,
    /// How many attempts so far.
    pub attempts: u32,
}

/// One row of the anime-titles dump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleRow {
    /// The series.
    pub series: AniDbSeriesId,
    /// 1 primary, 2 synonym, 3 short, 4 official.
    pub kind: u8,
    /// Language tag ("x-jat", "en", "ja", ...).
    pub lang: String,
    /// The title.
    pub title: String,
}

/// One name-search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleSearchHit {
    /// The series.
    pub series: AniDbSeriesId,
    /// The series' primary title, for display.
    pub title: String,
    /// The title/synonym the query actually matched.
    pub matched: String,
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
    fn known_users_upserts_and_filters_by_cutoff() {
        let storage = ServerStorage::open_in_memory().unwrap();
        storage.record_seen(&UserId::new("kim"), 100).unwrap();
        storage.record_seen(&UserId::new("nero"), 200).unwrap();
        // A later call for the same user wins (upsert), never regresses.
        storage.record_seen(&UserId::new("kim"), 300).unwrap();

        let all = storage.known_users(0).unwrap();
        assert_eq!(
            all.into_iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
            std::collections::BTreeMap::from([
                (UserId::new("kim"), 300),
                (UserId::new("nero"), 200),
            ])
        );

        // The 30-day-cutoff boundary: nero (last_seen 200) drops out once
        // the cutoff passes it, kim (300) stays.
        let recent = storage.known_users(201).unwrap();
        assert_eq!(recent, vec![(UserId::new("kim"), 300)]);
        let none = storage.known_users(301).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn anidb_queue_lifecycle() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let info = FileHashInfo {
            hash: hash(1),
            size: 1234,
            filename: "ep1.mkv".into(),
            mtime: None,
            series_hint: None,
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

    /// An enqueue learns the file's mtime, and a later enqueue only ever
    /// lowers it toward the oldest reported value (a `None` never clobbers
    /// a known one). `first_seen` is never reset by a re-enqueue.
    #[test]
    fn enqueue_learns_and_minimises_mtime() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let info = |mtime| FileHashInfo {
            hash: hash(1),
            size: 1,
            filename: "ep1.mkv".into(),
            mtime,
            series_hint: None,
        };
        let read_mtime = |s: &ServerStorage| s.due_lookups(i64::MAX, 10).unwrap()[0].info.mtime;

        // First sighting carries an mtime; first_seen is pinned to 100.
        storage.enqueue_lookup(&info(Some(5_000)), 100).unwrap();
        assert_eq!(read_mtime(&storage), Some(5_000));
        assert_eq!(
            storage.due_lookups(i64::MAX, 10).unwrap()[0].first_seen,
            100
        );

        // A re-enqueue with no mtime must not wipe the known one.
        storage.enqueue_lookup(&info(None), 200).unwrap();
        assert_eq!(read_mtime(&storage), Some(5_000));

        // An older mtime wins (min); first_seen stays put.
        storage.enqueue_lookup(&info(Some(1_000)), 300).unwrap();
        assert_eq!(read_mtime(&storage), Some(1_000));
        assert_eq!(
            storage.due_lookups(i64::MAX, 10).unwrap()[0].first_seen,
            100
        );

        // A newer mtime does not raise it back up.
        storage.enqueue_lookup(&info(Some(9_000)), 400).unwrap();
        assert_eq!(read_mtime(&storage), Some(1_000));
    }

    /// A row created before the mtime column existed (NULL mtime) learns
    /// its mtime when a client re-reports the file post-upgrade. This is
    /// the path by which the existing queue settles after deploy.
    #[test]
    fn enqueue_fills_mtime_for_a_preexisting_null_row() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let info = |mtime| FileHashInfo {
            hash: hash(7),
            size: 1,
            filename: "old.mkv".into(),
            mtime,
            series_hint: None,
        };
        storage.enqueue_lookup(&info(None), 100).unwrap();
        assert_eq!(
            storage.due_lookups(i64::MAX, 10).unwrap()[0].info.mtime,
            None
        );
        storage.enqueue_lookup(&info(Some(42)), 200).unwrap();
        assert_eq!(
            storage.due_lookups(i64::MAX, 10).unwrap()[0].info.mtime,
            Some(42)
        );
    }

    /// An enqueue learns the series hint and keeps the first non-null one:
    /// a row queued without a hint (e.g. a playlist-only request) picks one
    /// up when a holder later reports it, and a differing later hint doesn't
    /// overwrite it. The hint round-trips through `due_lookups`.
    #[test]
    fn enqueue_learns_series_hint() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let info = |hint: Option<&str>| FileHashInfo {
            hash: hash(3),
            size: 1,
            filename: "RahXephon - 01.mkv".into(),
            mtime: None,
            series_hint: hint.map(str::to_string),
        };
        let read_hint = |s: &ServerStorage| {
            s.due_lookups(i64::MAX, 10).unwrap()[0]
                .info
                .series_hint
                .clone()
        };

        // Queued without a hint first.
        storage.enqueue_lookup(&info(None), 100).unwrap();
        assert_eq!(read_hint(&storage), None);

        // A holder reports the containing folder: the row learns it.
        storage
            .enqueue_lookup(&info(Some("RahXephon")), 200)
            .unwrap();
        assert_eq!(read_hint(&storage).as_deref(), Some("RahXephon"));

        // A later, different hint does not overwrite the first.
        storage
            .enqueue_lookup(&info(Some("Something Else")), 300)
            .unwrap();
        assert_eq!(read_hint(&storage).as_deref(), Some("RahXephon"));
    }

    /// Regression: a successful lookup marks the queue settled (has_data,
    /// recheck in a week) durably, but its metadata write can be lost to a
    /// restart before the CRDT snapshot persists. Such an orphan — settled
    /// in the queue, no metadata in state — must be re-armed to due now;
    /// settled rows that *do* have metadata, and unsettled rows, are left
    /// untouched (2026-06-15).
    #[test]
    fn rearm_resets_settled_lookups_whose_metadata_was_lost() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let info = |i: u8, name: &str| FileHashInfo {
            hash: hash(i),
            size: 1,
            filename: name.into(),
            mtime: None,
            series_hint: None,
        };
        // hash(1): settled with data but metadata lost (the orphan).
        // hash(2): settled with data and metadata present (healthy).
        // hash(3): settled WITHOUT data (a known AniDB miss) — leave it.
        for (i, name) in [(1u8, "orphan.mkv"), (2, "healthy.mkv"), (3, "miss.mkv")] {
            storage.enqueue_lookup(&info(i, name), 100).unwrap();
        }
        storage
            .record_lookup_attempt(hash(1), 100, 700_000, true)
            .unwrap();
        storage
            .record_lookup_attempt(hash(2), 100, 700_000, true)
            .unwrap();
        storage
            .record_lookup_attempt(hash(3), 100, 700_000, false)
            .unwrap();

        // Only hash(2) actually has replicated metadata.
        let present = std::collections::BTreeSet::from([hash(2)]);
        let rearmed = storage
            .rearm_settled_without_metadata(&present, 200)
            .unwrap();
        assert_eq!(rearmed, vec!["orphan.mkv".to_string()]);

        // The orphan is due now and no longer claims data.
        let due = storage.due_lookups(200, 10).unwrap();
        let due_hashes: Vec<_> = due.iter().map(|e| e.info.hash).collect();
        assert!(due_hashes.contains(&hash(1)), "orphan must be due now");
        assert!(
            !due_hashes.contains(&hash(2)),
            "healthy row must stay settled"
        );
        assert!(
            !due_hashes.contains(&hash(3)),
            "no-data row must stay on its ladder"
        );
        let orphan = due.iter().find(|e| e.info.hash == hash(1)).unwrap();
        assert!(!orphan.has_data, "re-armed orphan must drop has_data");
    }

    #[test]
    fn anime_queue_lifecycle() {
        let storage = ServerStorage::open_in_memory().unwrap();
        let series = AniDbSeriesId(8692);
        storage.enqueue_anime(series, 100).unwrap();
        // Re-discovery keeps the existing schedule.
        storage.enqueue_anime(series, 999).unwrap();

        let due = storage.due_anime(100, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].series, series);
        assert_eq!(due[0].next_attempt, 100);

        // Failed attempt: retry later.
        storage.record_anime_attempt(series, 100, 200).unwrap();
        assert!(storage.due_anime(199, 10).unwrap().is_empty());
        assert_eq!(storage.due_anime(200, 10).unwrap()[0].attempts, 1);

        // Success settles the entry as a tombstone: never due again,
        // but still present so re-discovery stays a no-op.
        storage.record_anime_attempt(series, 200, NEVER).unwrap();
        assert!(storage.due_anime(i64::MAX - 1, 10).unwrap().is_empty());
        storage.enqueue_anime(series, 300).unwrap();
        assert!(storage.due_anime(i64::MAX - 1, 10).unwrap().is_empty());
    }

    #[test]
    fn rearm_resets_settled_anime_whose_relations_were_lost() {
        let storage = ServerStorage::open_in_memory().unwrap();
        // aid 1: settled (NEVER) but relations lost (the orphan).
        // aid 2: settled and relations present (healthy).
        // aid 3: a pending timeout retry (not settled) — must be left alone.
        for aid in [1u32, 2] {
            storage.enqueue_anime(AniDbSeriesId(aid), 100).unwrap();
            storage
                .record_anime_attempt(AniDbSeriesId(aid), 100, NEVER)
                .unwrap();
        }
        storage.enqueue_anime(AniDbSeriesId(3), 100).unwrap();
        storage
            .record_anime_attempt(AniDbSeriesId(3), 100, 700_000)
            .unwrap();

        // Only aid 2 actually has replicated relations.
        let present = std::collections::BTreeSet::from([AniDbSeriesId(2)]);
        let rearmed = storage
            .rearm_settled_anime_without_relations(&present, 200)
            .unwrap();
        assert_eq!(rearmed, vec![AniDbSeriesId(1)]);

        let due: Vec<_> = storage
            .due_anime(200, 10)
            .unwrap()
            .into_iter()
            .map(|e| e.series)
            .collect();
        assert!(due.contains(&AniDbSeriesId(1)), "orphan must be due now");
        assert!(
            !due.contains(&AniDbSeriesId(2)),
            "healthy row must stay settled"
        );
        assert!(
            !due.contains(&AniDbSeriesId(3)),
            "a pending (non-settled) row must not be disturbed"
        );
    }

    fn title(aid: u32, kind: u8, title: &str) -> TitleRow {
        TitleRow {
            series: AniDbSeriesId(aid),
            kind,
            lang: "x-jat".into(),
            title: title.into(),
        }
    }

    #[test]
    fn title_search_ranks_and_dedupes() {
        let mut storage = ServerStorage::open_in_memory().unwrap();
        storage
            .replace_titles(&[
                title(1, 1, "Gochuumon wa Usagi Desu ka?"),
                title(1, 3, "GochiUsa"),
                title(2, 1, "Gochuumon wa Usagi Desu ka??"),
                title(3, 1, "Frieren"),
                title(4, 1, "Sousou no Frieren"),
                title(4, 2, "Frieren: Beyond Journey's End"),
            ])
            .unwrap();

        // Informal short name finds the series, displayed by its
        // primary title.
        let hits = storage.search_titles("gochiusa", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].series, AniDbSeriesId(1));
        assert_eq!(hits[0].title, "Gochuumon wa Usagi Desu ka?");
        assert_eq!(hits[0].matched, "GochiUsa");

        // Exact match outranks the longer prefix match; one hit per
        // series even when several titles match.
        let hits = storage.search_titles("frieren", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].series, AniDbSeriesId(3));
        assert_eq!(hits[1].series, AniDbSeriesId(4));

        // Substring matches work too.
        let hits = storage.search_titles("usagi", 10).unwrap();
        assert_eq!(hits.len(), 2);

        // The limit applies after dedup.
        assert_eq!(storage.search_titles("usagi", 1).unwrap().len(), 1);

        // LIKE wildcards in the query are literal.
        assert!(storage.search_titles("%", 10).unwrap().is_empty());
        assert!(storage.search_titles("usagi_desu", 10).unwrap().is_empty());

        // A fresh dump replaces everything.
        storage.replace_titles(&[title(9, 1, "Other")]).unwrap();
        assert!(storage.search_titles("frieren", 10).unwrap().is_empty());
    }

    #[test]
    fn kv_round_trips() {
        let storage = ServerStorage::open_in_memory().unwrap();
        assert_eq!(storage.kv_get("titles_fetched_at").unwrap(), None);
        storage.kv_set("titles_fetched_at", "12345").unwrap();
        assert_eq!(
            storage.kv_get("titles_fetched_at").unwrap().as_deref(),
            Some("12345")
        );
        storage.kv_set("titles_fetched_at", "99").unwrap();
        assert_eq!(
            storage.kv_get("titles_fetched_at").unwrap().as_deref(),
            Some("99")
        );
    }
}
