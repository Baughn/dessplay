//! Synced-state persistence: the client's sibling `dessplay.sync.db`.
//!
//! The replicated CRDT snapshot lives in its own database next to the
//! main one (`dessplay.db` → `dessplay.sync.db`), split out 2026-08-21
//! after the tsugumi restore incident: the remedy for wedged or polluted
//! sync state used to be "delete the client database", which also cost
//! `hash_cache` — hours of re-hashing a TB-scale library. With the
//! split, discarding the replicated state (`dessplay --reset-sync`, or
//! `rm dessplay.sync.db*` as the manual fallback) touches nothing local.
//!
//! The state here is losslessly recoverable from the server (it is a
//! replica, the server is authoritative), which is exactly the property
//! the split keys on: everything in the *main* database is local-only
//! and irreplaceable-ish; everything here is disposable.
//!
//! **One-time move**: `SyncStorage::open` adopts the legacy `crdt_state`
//! row from the main database, then drops the legacy table — idempotent
//! and crash-safe (see [`SyncStorage::adopt_legacy_state`]). This is
//! deliberately not a main-DB migration; see the note on
//! `storage::MIGRATIONS`.
//!
//! **Locking**: every path that opens this database resolves it from the
//! main database path and already holds the `<db>.lock` instance lock
//! (run_interactive, run_headless, `--reset-sync`), so the sync database
//! needs no lock file of its own (see instance_lock.rs). The exception
//! is `--dump`, which is read-only here by construction: it uses
//! [`SyncStorage::open_at`] and never triggers the move.
//!
//! Same conventions as storage.rs: timestamps are caller-supplied unix
//! milliseconds (storage never reads the clock), and the struct wraps a
//! single non-`Sync` connection — the owning actor serializes access.

use std::path::{Path, PathBuf};

use dessplay_core::types::Epoch;
use dessplay_core::{CrdtState, StateSnapshot};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::storage::{BUSY_TIMEOUT, Result, StorageError, migrate};

/// The sync database's own schema. Append-only, versioned via `PRAGMA
/// user_version`, independent of the main database's migration list.
const MIGRATIONS: &[&str] = &[
    // v1 (2026-08-21): the crdt_state table, column-identical to the
    // legacy main-db table it replaces.
    "
    CREATE TABLE crdt_state (
        room     TEXT PRIMARY KEY,     -- single implicit room
        epoch    INTEGER NOT NULL,
        state    BLOB NOT NULL,        -- tagged-envelope postcard CrdtState
        saved_at INTEGER NOT NULL
    ) STRICT;
    ",
];

/// Persistence for the replicated CRDT snapshot, and nothing else.
pub struct SyncStorage {
    conn: Connection,
}

impl SyncStorage {
    /// The sync database path derived from the main database path:
    /// `dessplay.db` → sibling `dessplay.sync.db`. Deriving (rather than
    /// taking a second flag) keeps `--db` working unchanged and keeps
    /// the pair inside the same `<db>.lock` instance-lock scope.
    pub fn derive_path(db_path: &Path) -> PathBuf {
        match (db_path.file_stem(), db_path.extension()) {
            (Some(stem), Some(ext)) => {
                let mut name = stem.to_os_string();
                name.push(".sync.");
                name.push(ext);
                db_path.with_file_name(name)
            }
            (Some(stem), None) => {
                let mut name = stem.to_os_string();
                name.push(".sync.db");
                db_path.with_file_name(name)
            }
            (None, _) => db_path.with_file_name("dessplay.sync.db"),
        }
    }

    /// Open (creating and migrating as needed) the sync database derived
    /// from the *main* database path, then complete the one-time move of
    /// any legacy `crdt_state` row out of the main database. Parent
    /// directories are created. Callers must hold the instance lock for
    /// `db_path` — the move writes to the main database.
    pub fn open(db_path: &Path) -> Result<Self> {
        let sync_path = Self::derive_path(db_path);
        tracing::debug!(path = %sync_path.display(), "opening sync database");
        if let Some(parent) = sync_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::Corrupt(format!("creating {parent:?}: {e}")))?;
        }
        let this = Self::init(Connection::open(&sync_path)?)?;
        this.adopt_legacy_state(db_path)?;
        Ok(this)
    }

    /// Open the sync database at an explicit path, with no legacy move.
    /// For `--dump` (which must never mutate the main database — it runs
    /// without the instance lock, possibly beside a live instance) and
    /// tests.
    pub fn open_at(sync_path: &Path) -> Result<Self> {
        tracing::debug!(path = %sync_path.display(), "opening sync database (no move)");
        if let Some(parent) = sync_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::Corrupt(format!("creating {parent:?}: {e}")))?;
        }
        Self::init(Connection::open(sync_path)?)
    }

    /// An in-memory sync database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Two same-process connections can exist (the sync actor's, and a
        // concurrent `--dump`'s); same rationale as storage.rs.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        migrate(&conn, MIGRATIONS)?;
        Ok(Self { conn })
    }

    /// One-time move of the legacy `crdt_state` row from the main
    /// database (idempotent, crash-safe):
    ///
    /// 1. If this sync database already has a row, never copy — a stale
    ///    legacy row must not overwrite newer state.
    /// 2. Otherwise, if the main database has a `crdt_state` table with
    ///    rows, copy them in and commit.
    /// 3. Drop the legacy table only when the rows are known to be here
    ///    (or the table was empty). A failed copy propagates its error
    ///    *before* the drop; a crash between copy-commit and drop
    ///    re-enters via step 1's skip path and completes the drop.
    ///
    /// The main database is opened without `SQLITE_OPEN_CREATE`: a fresh
    /// install has no main database yet and must not grow one here.
    fn adopt_legacy_state(&self, main_db: &Path) -> Result<()> {
        if !main_db.exists() {
            return Ok(());
        }
        let main = Connection::open_with_flags(
            main_db,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        main.busy_timeout(BUSY_TIMEOUT)?;
        let table_exists = main
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'crdt_state'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !table_exists {
            return Ok(());
        }
        let already_here: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM crdt_state", [], |row| row.get(0))?;
        if already_here == 0 {
            let tx = self.conn.unchecked_transaction()?;
            let mut stmt = main.prepare("SELECT room, epoch, state, saved_at FROM crdt_state")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
            let mut moved = 0u32;
            for row in rows {
                let (room, epoch, state, saved_at) = row?;
                tx.execute(
                    "INSERT INTO crdt_state (room, epoch, state, saved_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![room, epoch, state, saved_at],
                )?;
                moved += 1;
            }
            tx.commit()?;
            if moved > 0 {
                tracing::info!(
                    rows = moved,
                    "legacy synced state moved into the sync database"
                );
            }
        } else {
            tracing::debug!("sync database already populated; legacy row (if any) is stale");
        }
        // Drop only when the state is known to be safe: either it now
        // lives here, or the legacy table never held any.
        let here: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM crdt_state", [], |row| row.get(0))?;
        let legacy: i64 =
            main.query_row("SELECT COUNT(*) FROM crdt_state", [], |row| row.get(0))?;
        if here > 0 || legacy == 0 {
            main.execute_batch("DROP TABLE crdt_state")?;
            tracing::info!("legacy crdt_state table dropped from the main database");
        }
        Ok(())
    }

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

    /// Discard the stored snapshot (`dessplay --reset-sync`). SQL, not
    /// file deletion: deleting the file under a live `-wal`/`-shm` pair
    /// would orphan them, and the schema survives for the next save.
    pub fn clear(&self) -> Result<()> {
        let rows = self.conn.execute("DELETE FROM crdt_state", [])?;
        tracing::info!(rows, "synced state cleared");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::test_support::{ClusterEvent, ScriptOp, run_cluster};

    use super::*;

    /// A nontrivial snapshot via the shared cluster generator.
    fn snapshot(epoch: u64) -> StateSnapshot {
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
        ]);
        StateSnapshot {
            epoch: Epoch(epoch),
            state: cluster.server,
        }
    }

    /// Build a *legacy* main database via raw SQL: `Storage` can no
    /// longer write (or even name) the crdt_state table, so fixtures for
    /// the move tests reproduce the old schema by hand.
    fn legacy_main_db(path: &Path, snapshot: Option<&StateSnapshot>) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE crdt_state (
                room     TEXT PRIMARY KEY,
                epoch    INTEGER NOT NULL,
                state    BLOB NOT NULL,
                saved_at INTEGER NOT NULL
            ) STRICT;",
        )
        .unwrap();
        if let Some(snapshot) = snapshot {
            conn.execute(
                "INSERT INTO crdt_state (room, epoch, state, saved_at)
                 VALUES ('default', ?1, ?2, 1000)",
                params![
                    snapshot.epoch.0 as i64,
                    snapshot.state.encode_snapshot().unwrap()
                ],
            )
            .unwrap();
        }
    }

    fn legacy_table_exists(path: &Path) -> bool {
        let conn = Connection::open(path).unwrap();
        conn.query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'crdt_state'",
            [],
            |_| Ok(()),
        )
        .optional()
        .unwrap()
        .is_some()
    }

    #[test]
    fn derive_path_makes_a_sibling() {
        assert_eq!(
            SyncStorage::derive_path(Path::new("/x/dessplay.db")),
            Path::new("/x/dessplay.sync.db")
        );
        // `--db` with a different extension keeps it.
        assert_eq!(
            SyncStorage::derive_path(Path::new("/x/mine.sqlite")),
            Path::new("/x/mine.sync.sqlite")
        );
        // No extension at all still yields a distinct sibling.
        assert_eq!(
            SyncStorage::derive_path(Path::new("/x/state")),
            Path::new("/x/state.sync.db")
        );
    }

    #[test]
    fn fresh_install_has_no_state_and_creates_no_main_db() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("dessplay.db");
        let storage = SyncStorage::open(&main).unwrap();
        assert!(storage.load_state().unwrap().is_none());
        // The move must not conjure a main database out of nothing.
        assert!(!main.exists(), "open() must not create the main database");
        assert!(SyncStorage::derive_path(&main).exists());
    }

    #[test]
    fn snapshot_round_trips_through_db() {
        let storage = SyncStorage::open_in_memory().unwrap();
        assert!(storage.load_state().unwrap().is_none());

        let snapshot = snapshot(42);
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
        let storage = SyncStorage::open_in_memory().unwrap();
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
    fn clear_discards_the_snapshot_but_keeps_the_schema() {
        let storage = SyncStorage::open_in_memory().unwrap();
        storage.save_state(&snapshot(7), 1000).unwrap();
        storage.clear().unwrap();
        assert!(storage.load_state().unwrap().is_none());
        // The table survives (clear is SQL, not file deletion): a
        // subsequent save must work without re-migrating.
        storage.save_state(&snapshot(8), 2000).unwrap();
        assert_eq!(storage.load_state().unwrap().unwrap().epoch, Epoch(8));
    }

    // ---- The one-time move (the crash-safety contract, plan Phase 3).

    #[test]
    fn legacy_row_is_moved_once_and_the_table_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("dessplay.db");
        let expected = snapshot(9);
        legacy_main_db(&main, Some(&expected));

        {
            let storage = SyncStorage::open(&main).unwrap();
            let loaded = storage.load_state().unwrap().unwrap();
            assert_eq!(loaded, expected, "the legacy row must be adopted");
        }
        assert!(
            !legacy_table_exists(&main),
            "the legacy table must be dropped after a good copy"
        );

        // Re-open: idempotent (no table left, state intact).
        let storage = SyncStorage::open(&main).unwrap();
        assert_eq!(storage.load_state().unwrap().unwrap(), expected);
    }

    #[test]
    fn existing_sync_row_wins_over_a_stale_legacy_row() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("dessplay.db");
        let stale = snapshot(3);
        let newer = snapshot(12);
        legacy_main_db(&main, Some(&stale));
        // The sync database already carries newer state (a prior run of
        // the split code wrote it; the legacy row is a leftover).
        SyncStorage::open_at(&SyncStorage::derive_path(&main))
            .unwrap()
            .save_state(&newer, 2000)
            .unwrap();

        let storage = SyncStorage::open(&main).unwrap();
        assert_eq!(
            storage.load_state().unwrap().unwrap(),
            newer,
            "a stale legacy row must never overwrite newer sync state"
        );
        assert!(
            !legacy_table_exists(&main),
            "the stale legacy table must still be dropped"
        );
    }

    #[test]
    fn crash_between_copy_and_drop_completes_on_reopen() {
        // The crash window: the copy committed into sync.db, the process
        // died before the DROP. On re-entry the skip path (sync.db has
        // the row) must still complete the drop.
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("dessplay.db");
        let state = snapshot(5);
        legacy_main_db(&main, Some(&state));
        SyncStorage::open_at(&SyncStorage::derive_path(&main))
            .unwrap()
            .save_state(&state, 1000)
            .unwrap();

        let storage = SyncStorage::open(&main).unwrap();
        assert_eq!(storage.load_state().unwrap().unwrap(), state);
        assert!(
            !legacy_table_exists(&main),
            "re-entry after the crash window must complete the drop"
        );
    }

    #[test]
    fn empty_legacy_table_is_dropped() {
        // Fresh main databases still create the (empty) v1 crdt_state
        // table — migrations are append-only. The first SyncStorage::open
        // retires it.
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("dessplay.db");
        legacy_main_db(&main, None);

        let storage = SyncStorage::open(&main).unwrap();
        assert!(storage.load_state().unwrap().is_none());
        assert!(
            !legacy_table_exists(&main),
            "an empty legacy table is safe to drop"
        );
    }
}
