//! Process-wide single-instance guard.
//!
//! Two dessplay processes that open the same SQLite database (and download
//! cache) concurrently corrupt each other's state. The motivating incident: a
//! client and a seeder launched from the same home directory with no `--db` /
//! `--cache-dir` overrides — both resolve the same default paths
//! (`Storage::default_path`, `download_cache_dir`) and fight over the same
//! `dessplay.db` (WAL, no `busy_timeout`) and the same hash-named cache files.
//!
//! We refuse that up front. At startup each process takes an *exclusive
//! advisory lock* (flock) on a lock file beside the database and inside the
//! cache directory; a second instance bound to either path fails to start with
//! an actionable error. Advisory locks are released when the handle closes —
//! on clean exit *or* a crash — so there is no stale-lock file to reap, unlike
//! a pidfile.
//!
//! Running a second, independent instance stays possible: give it its own
//! `--db` and `--cache-dir`, and the lock paths no longer collide.
//!
//! The `<db>.lock` scope also covers the sibling sync database
//! (`dessplay.sync.db`, sync_storage.rs) without a lock file of its own:
//! its path is *derived* from the main database path, and every writer —
//! run_interactive, run_headless, `--reset-sync` — acquires this lock on
//! that same main path first. (`--dump` opens the sync database without
//! the lock, but is read-only there by construction.)
//!
//! Implemented on std's native file locking (`File::try_lock`, stabilized in
//! Rust 1.89) — no third-party crate needed.

use std::ffi::OsString;
use std::fs::{File, TryLockError};
use std::path::Path;

/// Exclusive ownership of a database (and optionally a cache directory) for
/// the lifetime of the process. Holding the locked file handles open is what
/// keeps the locks held; dropping this releases them.
#[must_use = "the locks are released as soon as this guard is dropped"]
pub struct InstanceLock {
    _files: Vec<File>,
}

/// Acquire exclusive advisory locks guarding `db_path` and, when given,
/// `cache_dir`. Returns a human-readable error when another dessplay instance
/// already holds either lock (or a lock file cannot be created).
pub fn acquire(db_path: &Path, cache_dir: Option<&Path>) -> Result<InstanceLock, String> {
    let mut files = Vec::with_capacity(2);

    // Lock a sibling `<db>.lock` rather than the database file itself: the
    // database is reopened several times within one process (settings, sync,
    // file actors), and flock treats separate descriptors as conflicting — so
    // locking the db file directly would deadlock us against ourselves.
    let db_lock = append_ext(db_path, "lock");
    files.push(lock_file(
        &db_lock,
        &format!(
            "another dessplay instance is already using the database {}. \
             To run a second instance, give it its own --db (and --cache-dir).",
            db_path.display()
        ),
    )?);

    if let Some(cache_dir) = cache_dir {
        let cache_lock = cache_dir.join(".lock");
        files.push(lock_file(
            &cache_lock,
            &format!(
                "another dessplay instance is already using the download cache {}. \
                 To run a second instance, give it its own --cache-dir (and --db).",
                cache_dir.display()
            ),
        )?);
    }

    Ok(InstanceLock { _files: files })
}

/// Create `lock_path` (and its parent) and take an exclusive non-blocking
/// flock on it. `contended_msg` is returned verbatim when the lock is already
/// held by another process.
fn lock_file(lock_path: &Path, contended_msg: &str) -> Result<File, String> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let file = File::create(lock_path)
        .map_err(|e| format!("creating lock file {}: {e}", lock_path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(contended_msg.to_string()),
        Err(TryLockError::Error(e)) => Err(format!("locking {}: {e}", lock_path.display())),
    }
}

/// Append `.ext` to `path`, preserving any existing extension
/// (`dessplay.db` -> `dessplay.db.lock`, not `dessplay.lock`).
fn append_ext(path: &Path, ext: &str) -> std::path::PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    s.into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn second_acquire_is_rejected_while_held() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("dessplay.db");

        let first = acquire(&db, None).expect("first acquire succeeds");
        // A second process (here, a second handle) bound to the same db must
        // be refused while the first holds it.
        assert!(
            acquire(&db, None).is_err(),
            "second acquire must fail while the first is held"
        );

        drop(first);
        // Once released, the path is free again.
        let _second = acquire(&db, None).expect("re-acquire after release succeeds");
    }

    #[test]
    fn cache_lock_is_independent_of_db() {
        let dir = TempDir::new().unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&cache).unwrap();

        // Two different databases lock fine simultaneously...
        let a = acquire(&dir.path().join("a.db"), Some(&cache)).expect("first holds db+cache");
        // ...but a *shared cache dir* is rejected even with a different db.
        assert!(
            acquire(&dir.path().join("b.db"), Some(&cache)).is_err(),
            "a shared cache dir must be refused even when db paths differ"
        );

        drop(a);
        let _b = acquire(&dir.path().join("b.db"), Some(&cache)).expect("cache free after release");
    }

    #[test]
    fn append_ext_preserves_existing_extension() {
        assert_eq!(
            append_ext(Path::new("/x/dessplay.db"), "lock"),
            Path::new("/x/dessplay.db.lock")
        );
    }
}
