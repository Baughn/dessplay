//! The file actor: everything that touches local files (Phase 9).
//!
//! Owns the hash cache (ed2k roots + per-block hashes, validated by
//! mtime/size), manual mappings, download-cache bookkeeping
//! (retention/eviction), watch history writes, and the placeholder
//! renderer. Phase 9B adds download coordination.
//!
//! Heavy IO (directory scans, hashing, PNG rendering) runs in
//! `spawn_blocking` subtasks so the inbox stays live — a stuck hash
//! must never stop a resolve or an eviction pass. Completions return
//! through an internal channel; the actor task itself only does quick
//! SQLite bookkeeping.
//!
//! Absorbs Phase 7's `matcher` module: resolution is now hash-cache
//! aware, so unwatched playlist entries are not re-hashed every
//! session, and a file whose mtime changed is re-hashed exactly once
//! (design.md, File Matching / Content Hash).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dessplay_core::hash::{Ed2kFileHash, ed2k_hash_reader};
use dessplay_core::types::{AniDbSeriesId, Ed2kHash};
use tokio::sync::mpsc;

use crate::actors::network::Clock;
use crate::config::CacheRetention;
use crate::storage::{CacheEntry, SeriesKey, Storage, WatchRecord};

/// What resolving a playlist entry against the media roots found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// A file with the right name *and* the right contents (or a
    /// manual mapping, which is exempt from verification by design).
    Verified(PathBuf),
    /// Name matches somewhere, but no candidate's contents hash to the
    /// playlist key (a different encode — blocks playback, design.md
    /// "Before Playback Starts").
    HashMismatch(PathBuf),
    /// No file with that name under any root.
    NotFound,
}

/// A finished background hash for a playlist add.
#[derive(Debug)]
pub struct HashedAdd {
    /// The file that was hashed.
    pub path: PathBuf,
    /// Playlist anchor for the add.
    pub after: Option<Ed2kHash>,
    /// The hash, or why not.
    pub result: std::io::Result<Ed2kFileHash>,
}

/// Progress of background playlist-add hashing — the design's
/// no-silent-work rule: long-running operations always show progress in
/// the UI (design.md, UI Principles).
#[derive(Debug)]
pub enum HashEvent {
    /// Hashing is under way (sent at start and periodically after).
    Progress {
        /// The file being hashed.
        path: PathBuf,
        /// Bytes read so far.
        done_bytes: u64,
        /// File size (0 when unknowable).
        total_bytes: u64,
    },
    /// Hashing finished (also on error — the UI row goes away either
    /// way).
    Done(HashedAdd),
}

/// Commands into the actor.
#[derive(Debug)]
pub enum FileCommand {
    /// Find a local copy of a playlist entry: manual mapping first,
    /// then a media-root scan with hash-cache-backed verification.
    Resolve {
        /// Playlist key to verify against.
        file: Ed2kHash,
        /// Filename to search for.
        filename: String,
    },
    /// Hash a local file for a playlist add (progress + Done events).
    HashAdd {
        /// The file to hash.
        path: PathBuf,
        /// Playlist anchor.
        after: Option<Ed2kHash>,
    },
    /// The user manually mapped `file` to `path`. Persisted, and
    /// resolves Verified immediately (no hash check by design).
    SetManualMapping {
        /// The playlist entry.
        file: Ed2kHash,
        /// The chosen local file.
        path: PathBuf,
        /// Remember the directory for this series' next mapping.
        series: Option<SeriesKey>,
    },
    /// Record a personally-watched file (the 85% rule crossed).
    RecordWatched(WatchRecord),
    /// Is this series known (any personal watch history)?
    CheckSeriesKnown {
        /// The playlist entry that triggered the check.
        file: Ed2kHash,
        /// The series id, when metadata has one (enables the synced
        /// NotWatching preference downstream).
        series: Option<AniDbSeriesId>,
        /// History lookup key (id, or parsed-name fallback).
        key: SeriesKey,
    },
    /// Render the not-watching placeholder PNG.
    RenderPlaceholder {
        /// The file the placeholder stands in for.
        file: Ed2kHash,
        /// Text lines (filename, explanation, session status).
        lines: Vec<String>,
    },
    /// Move a cached download into the library at `dest`.
    Archive {
        /// The cached file.
        file: Ed2kHash,
        /// Full destination path (download root / series / season /
        /// filename, computed by the caller from synced metadata).
        dest: PathBuf,
    },
    /// Eviction pass (startup and EOF-advance).
    RunEviction {
        /// Never evicted: now-playing and queued unwatched entries.
        protected: HashSet<Ed2kHash>,
        /// Group watched flags (an entry behind the group's progress
        /// is evictable even if never personally watched).
        group_watched: HashSet<Ed2kHash>,
    },
    /// Media roots changed (settings save).
    SetMediaRoots(Vec<PathBuf>),
    /// Retention policy changed (settings save).
    SetRetention(CacheRetention),
}

/// Events out of the actor.
#[derive(Debug)]
pub enum FileOutput {
    /// A resolve finished.
    Resolved {
        /// The playlist entry.
        file: Ed2kHash,
        /// What was found.
        resolution: Resolution,
    },
    /// Playlist-add hashing progress or completion.
    Hash(HashEvent),
    /// Answer to [`FileCommand::CheckSeriesKnown`].
    SeriesKnown {
        /// The playlist entry that triggered the check.
        file: Ed2kHash,
        /// Echoed series id.
        series: Option<AniDbSeriesId>,
        /// Whether any file of the series was ever watched here.
        known: bool,
    },
    /// The placeholder PNG is on disk.
    PlaceholderReady {
        /// The file it stands in for.
        file: Ed2kHash,
        /// Where the PNG was written.
        path: PathBuf,
    },
    /// An archive finished.
    Archived {
        /// The cached file.
        file: Ed2kHash,
        /// New library path, or why not.
        result: Result<PathBuf, String>,
    },
    /// An eviction pass deleted these cached files.
    Evicted {
        /// The evicted files.
        files: Vec<Ed2kHash>,
    },
}

/// Everything the actor needs at spawn.
pub struct FileConfig {
    /// Bookkeeping storage (hash cache, watch history, cache entries,
    /// manual mappings). Its own connection; WAL handles concurrency.
    pub storage: Storage,
    /// Media roots, in priority order.
    pub media_roots: Vec<PathBuf>,
    /// Download-cache retention policy.
    pub retention: CacheRetention,
    /// Download cache directory (placeholder PNG home; 9B downloads).
    pub cache_dir: PathBuf,
    /// Unix-millis clock (timestamps for bookkeeping rows).
    pub clock: Clock,
}

/// Completions from blocking subtasks.
enum Done {
    Resolved {
        file: Ed2kHash,
        resolution: Resolution,
        /// Hashes computed along the way, to commit to the cache.
        fresh: Vec<(PathBuf, i64, Ed2kFileHash)>,
    },
    Hashed {
        add: HashedAdd,
        /// The file's mtime at hash time (cache validity key).
        mtime: Option<i64>,
    },
    Placeholder {
        file: Ed2kHash,
        result: std::io::Result<PathBuf>,
    },
}

/// Run the actor until the command channel closes.
pub async fn run(
    config: FileConfig,
    mut commands: mpsc::Receiver<FileCommand>,
    out: mpsc::Sender<FileOutput>,
) {
    let (done_tx, mut done_rx) = mpsc::channel::<Done>(64);
    let mut actor = match Actor::new(config, out, done_tx) {
        Ok(actor) => actor,
        Err(e) => {
            tracing::error!("file actor failed to initialize: {e}");
            return;
        }
    };
    loop {
        tokio::select! {
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { break };
                actor.on_command(cmd).await;
            }
            done = done_rx.recv() => {
                // We hold a done_tx clone, so this only closes when we
                // drop it — i.e. never before the loop breaks.
                let Some(done) = done else { break };
                actor.on_done(done).await;
            }
        }
    }
    tracing::debug!("file actor exiting");
}

struct Actor {
    storage: Storage,
    media_roots: Vec<PathBuf>,
    retention: CacheRetention,
    cache_dir: PathBuf,
    clock: Clock,
    /// In-memory hash cache, shared with blocking subtasks as an
    /// immutable snapshot (replaced on change — changes are rare
    /// relative to lookups).
    hash_cache: Arc<HashMap<PathBuf, (i64, Ed2kFileHash)>>,
    /// Manual mappings (hash → user-picked path); checked before the
    /// matcher and exempt from hash verification by design.
    manual: HashMap<Ed2kHash, PathBuf>,
    /// Paths currently being hashed (dedupes impatient re-adds).
    hashing: HashSet<PathBuf>,
    out: mpsc::Sender<FileOutput>,
    done_tx: mpsc::Sender<Done>,
}

/// A file's mtime in unix millis (the hash-cache validity key).
fn mtime_millis(metadata: &std::fs::Metadata) -> Option<i64> {
    let mtime = metadata.modified().ok()?;
    match mtime.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).ok(),
        // Pre-epoch mtimes exist on weird filesystems; represent as 0.
        Err(_) => Some(0),
    }
}

impl Actor {
    fn new(
        config: FileConfig,
        out: mpsc::Sender<FileOutput>,
        done_tx: mpsc::Sender<Done>,
    ) -> Result<Self, crate::storage::StorageError> {
        let started = std::time::Instant::now();
        let hash_cache: HashMap<PathBuf, (i64, Ed2kFileHash)> = config
            .storage
            .hash_cache()?
            .into_iter()
            .map(|row| (row.path, (row.mtime, row.hash)))
            .collect();
        let manual: HashMap<Ed2kHash, PathBuf> =
            config.storage.manual_mappings()?.into_iter().collect();
        tracing::debug!(
            cached_hashes = hash_cache.len(),
            manual_mappings = manual.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "file actor ready"
        );
        Ok(Actor {
            storage: config.storage,
            media_roots: config.media_roots,
            retention: config.retention,
            cache_dir: config.cache_dir,
            clock: config.clock,
            hash_cache: Arc::new(hash_cache),
            manual,
            hashing: HashSet::new(),
            out,
            done_tx,
        })
    }

    async fn on_command(&mut self, cmd: FileCommand) {
        match cmd {
            FileCommand::Resolve { file, filename } => self.resolve(file, filename).await,
            FileCommand::HashAdd { path, after } => self.hash_add(path, after).await,
            FileCommand::SetManualMapping { file, path, series } => {
                self.set_manual_mapping(file, path, series).await;
            }
            FileCommand::RecordWatched(record) => {
                tracing::info!(filename = %record.filename, "marking personally watched (85%)");
                if let Err(e) = self.storage.record_watched(&record) {
                    tracing::error!("recording watch history: {e}");
                }
            }
            FileCommand::CheckSeriesKnown { file, series, key } => {
                let known = self.storage.series_known(&key).unwrap_or_else(|e| {
                    tracing::error!("series_known lookup: {e}");
                    // Fail safe: an unknown DB error must not silently
                    // mark a series NotWatching.
                    true
                });
                let _ = self
                    .out
                    .send(FileOutput::SeriesKnown {
                        file,
                        series,
                        known,
                    })
                    .await;
            }
            FileCommand::RenderPlaceholder { file, lines } => {
                let path = self.cache_dir.join("placeholder.png");
                let done_tx = self.done_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let result = crate::placeholder::render_to(&path, &lines).map(|()| path);
                    let _ = done_tx.blocking_send(Done::Placeholder { file, result });
                });
            }
            FileCommand::Archive { file, dest } => self.archive(file, dest).await,
            FileCommand::RunEviction {
                protected,
                group_watched,
            } => self.run_eviction(&protected, &group_watched).await,
            FileCommand::SetMediaRoots(roots) => self.media_roots = roots,
            FileCommand::SetRetention(retention) => self.retention = retention,
        }
    }

    async fn on_done(&mut self, done: Done) {
        match done {
            Done::Resolved {
                file,
                resolution,
                fresh,
            } => {
                self.commit_fresh_hashes(fresh);
                let _ = self
                    .out
                    .send(FileOutput::Resolved { file, resolution })
                    .await;
            }
            Done::Hashed { add, mtime } => {
                self.hashing.remove(&add.path);
                if let (Ok(hash), Some(mtime)) = (&add.result, mtime) {
                    self.commit_fresh_hashes(vec![(add.path.clone(), mtime, hash.clone())]);
                }
                let _ = self.out.send(FileOutput::Hash(HashEvent::Done(add))).await;
            }
            Done::Placeholder { file, result } => match result {
                Ok(path) => {
                    let _ = self
                        .out
                        .send(FileOutput::PlaceholderReady { file, path })
                        .await;
                }
                Err(e) => tracing::error!("placeholder render failed: {e}"),
            },
        }
    }

    /// Commit freshly-computed hashes to the in-memory cache and SQLite.
    fn commit_fresh_hashes(&mut self, fresh: Vec<(PathBuf, i64, Ed2kFileHash)>) {
        if fresh.is_empty() {
            return;
        }
        let now = (self.clock)() as i64;
        let mut cache = (*self.hash_cache).clone();
        for (path, mtime, hash) in fresh {
            if let Err(e) = self.storage.upsert_hash_cache(&path, mtime, &hash, now) {
                tracing::error!("persisting hash cache: {e}");
            }
            cache.insert(path, (mtime, hash));
        }
        self.hash_cache = Arc::new(cache);
    }

    async fn resolve(&mut self, file: Ed2kHash, filename: String) {
        // Manual mappings skip the matcher *and* hash verification —
        // the user explicitly chose that file (design.md).
        if let Some(path) = self.manual.get(&file) {
            if path.is_file() {
                let _ = self
                    .out
                    .send(FileOutput::Resolved {
                        file,
                        resolution: Resolution::Verified(path.clone()),
                    })
                    .await;
                return;
            }
            tracing::info!(path = %path.display(), "manual mapping points at nothing; re-matching");
        }
        let roots = self.media_roots.clone();
        let cache = Arc::clone(&self.hash_cache);
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let (resolution, fresh) = resolve_with_cache(&filename, file, &roots, &cache);
            tracing::debug!(
                filename,
                elapsed_ms = started.elapsed().as_millis() as u64,
                fresh_hashes = fresh.len(),
                ?resolution,
                "file resolution finished"
            );
            let _ = done_tx.blocking_send(Done::Resolved {
                file,
                resolution,
                fresh,
            });
        });
    }

    async fn hash_add(&mut self, path: PathBuf, after: Option<Ed2kHash>) {
        if self.hashing.contains(&path) {
            tracing::debug!(path = %path.display(), "already hashing; ignoring re-add");
            return;
        }
        // Cache hit: the add is instant, no progress overlay needed.
        if let Ok(metadata) = std::fs::metadata(&path)
            && let Some(mtime) = mtime_millis(&metadata)
            && let Some((cached_mtime, hash)) = self.hash_cache.get(&path)
            && *cached_mtime == mtime
            && hash.size_bytes == metadata.len()
        {
            tracing::debug!(path = %path.display(), "playlist add served from hash cache");
            let _ = self
                .out
                .send(FileOutput::Hash(HashEvent::Done(HashedAdd {
                    path,
                    after,
                    result: Ok(hash.clone()),
                })))
                .await;
            return;
        }
        self.hashing.insert(path.clone());
        tracing::info!(path = %path.display(), "hashing for playlist add");
        let out = self.out.clone();
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let metadata = std::fs::metadata(&path).ok();
            let total_bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = metadata.as_ref().and_then(mtime_millis);
            // The opening progress event puts the file on screen
            // immediately (the no-silent-work rule).
            let _ = out.try_send(FileOutput::Hash(HashEvent::Progress {
                path: path.clone(),
                done_bytes: 0,
                total_bytes,
            }));
            let result = std::fs::File::open(&path).and_then(|f| {
                ed2k_hash_reader(ProgressReader {
                    inner: f,
                    path: path.clone(),
                    total_bytes,
                    done_bytes: 0,
                    last_reported: 0,
                    events: out.clone(),
                })
            });
            tracing::info!(
                path = %path.display(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                ok = result.is_ok(),
                "hash finished"
            );
            let _ = done_tx.blocking_send(Done::Hashed {
                add: HashedAdd {
                    path,
                    after,
                    result,
                },
                mtime,
            });
        });
    }

    async fn set_manual_mapping(
        &mut self,
        file: Ed2kHash,
        path: PathBuf,
        series: Option<SeriesKey>,
    ) {
        let now = (self.clock)() as i64;
        if let Err(e) = self.storage.set_manual_mapping(file, &path, now) {
            tracing::error!("persisting manual mapping: {e}");
        }
        if let Some(key) = series
            && let Some(dir) = path.parent()
            && let Err(e) = self.storage.set_series_map_dir(&key, dir, now)
        {
            tracing::error!("persisting series map dir: {e}");
        }
        tracing::info!(path = %path.display(), "manual mapping set");
        self.manual.insert(file, path.clone());
        let _ = self
            .out
            .send(FileOutput::Resolved {
                file,
                resolution: Resolution::Verified(path),
            })
            .await;
    }

    async fn archive(&mut self, file: Ed2kHash, dest: PathBuf) {
        let result = self.archive_inner(file, &dest);
        if let Ok(new_path) = &result {
            tracing::info!(path = %new_path.display(), "archived cached file into the library");
        }
        let _ = self.out.send(FileOutput::Archived { file, result }).await;
    }

    fn archive_inner(&mut self, file: Ed2kHash, dest: &Path) -> Result<PathBuf, String> {
        let entries = self.storage.cache_entries().map_err(|e| e.to_string())?;
        let entry = entries
            .iter()
            .find(|entry| entry.hash == file)
            .ok_or("not a cached download")?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
        }
        move_file(&entry.path, dest).map_err(|e| e.to_string())?;
        self.storage
            .remove_cache_entry(file)
            .map_err(|e| e.to_string())?;
        // The hash is content-derived and unchanged; re-key the cache
        // row to the new path (mtime may differ after a cross-device
        // copy).
        if let Err(e) = self.storage.remove_hash_cache(&entry.path) {
            tracing::error!("hash-cache cleanup after archive: {e}");
        }
        let mut cache = (*self.hash_cache).clone();
        if let Some((_, hash)) = cache.remove(&entry.path) {
            let now = (self.clock)() as i64;
            if let Ok(metadata) = std::fs::metadata(dest)
                && let Some(mtime) = mtime_millis(&metadata)
            {
                if let Err(e) = self.storage.upsert_hash_cache(dest, mtime, &hash, now) {
                    tracing::error!("hash-cache re-key after archive: {e}");
                }
                cache.insert(dest.to_path_buf(), (mtime, hash));
            }
        }
        self.hash_cache = Arc::new(cache);
        Ok(dest.to_path_buf())
    }

    async fn run_eviction(
        &mut self,
        protected: &HashSet<Ed2kHash>,
        group_watched: &HashSet<Ed2kHash>,
    ) {
        let now = (self.clock)() as i64;
        let entries = match self.storage.cache_entries() {
            Ok(entries) => entries,
            Err(e) => {
                tracing::error!("listing cache entries: {e}");
                return;
            }
        };
        let mut evicted = Vec::new();
        for entry in entries {
            let watched = group_watched.contains(&entry.hash)
                || matches!(self.storage.watched(entry.hash), Ok(Some(_)));
            if !evictable(now, self.retention, &entry, watched, protected.contains(&entry.hash)) {
                continue;
            }
            tracing::info!(path = %entry.path.display(), "evicting watched cached file");
            if let Err(e) = std::fs::remove_file(&entry.path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::error!(path = %entry.path.display(), "evicting: {e}");
                continue;
            }
            if let Err(e) = self.storage.remove_cache_entry(entry.hash) {
                tracing::error!("cache bookkeeping: {e}");
            }
            if let Err(e) = self.storage.remove_hash_cache(&entry.path) {
                tracing::error!("hash-cache cleanup: {e}");
            }
            let mut cache = (*self.hash_cache).clone();
            cache.remove(&entry.path);
            self.hash_cache = Arc::new(cache);
            evicted.push(entry.hash);
        }
        if !evicted.is_empty() {
            let _ = self
                .out
                .send(FileOutput::Evicted { files: evicted })
                .await;
        }
    }
}

/// The eviction rule (design.md, Download Cache and Retention): a
/// cached file is evicted iff it is watched (personally, or behind the
/// group), not protected (now-playing / queued unwatched), and its last
/// access is older than the retention window.
pub fn evictable(
    now: i64,
    retention: CacheRetention,
    entry: &CacheEntry,
    watched: bool,
    protected: bool,
) -> bool {
    if protected || !watched {
        return false;
    }
    match retention {
        CacheRetention::AfterWatch => true,
        CacheRetention::Keep(window) => {
            now.saturating_sub(entry.last_access) >= window.as_millis() as i64
        }
        CacheRetention::Infinite => false,
    }
}

/// Move a file, falling back to copy+delete across filesystems (the
/// cache dir and the download root are often different mounts).
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
        Err(e) => Err(e),
    }
}

/// Find a local copy of `filename` under `roots`, verified against
/// `expected`. Candidates are checked in root order (the first root is
/// the download target, most likely to be current). A candidate whose
/// (mtime, size) match a cache row is trusted without re-reading;
/// anything else is hashed and reported back for caching.
fn resolve_with_cache(
    filename: &str,
    expected: Ed2kHash,
    roots: &[PathBuf],
    cache: &HashMap<PathBuf, (i64, Ed2kFileHash)>,
) -> (Resolution, Vec<(PathBuf, i64, Ed2kFileHash)>) {
    let mut fresh = Vec::new();
    let mut mismatch = None;
    for candidate in find_by_name(filename, roots) {
        let metadata = match std::fs::metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(e) => {
                tracing::debug!(path = %candidate.display(), "unreadable candidate: {e}");
                continue;
            }
        };
        let mtime = mtime_millis(&metadata);
        let cached_root = mtime.and_then(|mtime| {
            cache.get(&candidate).and_then(|(cached_mtime, hash)| {
                (*cached_mtime == mtime && hash.size_bytes == metadata.len())
                    .then_some(hash.root)
            })
        });
        let root = match cached_root {
            Some(root) => root,
            None => {
                // Cache miss or stale mtime: hash for real, once.
                match std::fs::File::open(&candidate).and_then(ed2k_hash_reader) {
                    Ok(hashed) => {
                        let root = hashed.root;
                        if let Some(mtime) = mtime {
                            fresh.push((candidate.clone(), mtime, hashed));
                        }
                        root
                    }
                    Err(e) => {
                        tracing::debug!(path = %candidate.display(), "unreadable candidate: {e}");
                        continue;
                    }
                }
            }
        };
        if root == expected {
            return (Resolution::Verified(candidate), fresh);
        }
        tracing::debug!(path = %candidate.display(), "filename match, hash mismatch");
        mismatch.get_or_insert(candidate);
    }
    let resolution = match mismatch {
        Some(path) => Resolution::HashMismatch(path),
        None => Resolution::NotFound,
    };
    (resolution, fresh)
}

/// Every file named exactly `filename` under the roots, in breadth-first
/// root order. Symlinked directories are skipped (cycle safety).
fn find_by_name(filename: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        let mut queue = std::collections::VecDeque::from([root.clone()]);
        while let Some(dir) = queue.pop_front() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::debug!(dir = %dir.display(), "unreadable directory: {e}");
                    continue;
                }
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    queue.push_back(path);
                } else if file_type.is_file() && entry.file_name().to_string_lossy() == filename {
                    found.push(path);
                }
            }
        }
    }
    found
}

/// Emit a progress event at most every this many bytes.
const HASH_PROGRESS_STRIDE: u64 = 64 * 1024 * 1024;

/// A reader that reports cumulative progress through a lossy channel
/// (a dropped update is fine; the next one supersedes it).
struct ProgressReader<R> {
    inner: R,
    path: PathBuf,
    total_bytes: u64,
    done_bytes: u64,
    last_reported: u64,
    events: mpsc::Sender<FileOutput>,
}

impl<R: std::io::Read> std::io::Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done_bytes += n as u64;
        if self.done_bytes - self.last_reported >= HASH_PROGRESS_STRIDE {
            self.last_reported = self.done_bytes;
            let _ = self.events.try_send(FileOutput::Hash(HashEvent::Progress {
                path: self.path.clone(),
                done_bytes: self.done_bytes,
                total_bytes: self.total_bytes,
            }));
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::time::Duration;

    use dessplay_core::hash::ed2k_hash_bytes;

    use super::*;

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn write(dir: &Path, rel: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    // ---- resolve_with_cache (pure-ish; real tempdir filesystem).

    #[test]
    fn finds_a_verified_file_in_a_nested_directory() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        let path = write(root.path(), "Frieren/Season 1/ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;
        let (resolution, fresh) = resolve_with_cache(
            "ep1.mkv",
            expected,
            &[root.path().to_path_buf()],
            &HashMap::new(),
        );
        assert_eq!(resolution, Resolution::Verified(path.clone()));
        // The hash it computed comes back for caching.
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].0, path);
        assert_eq!(fresh[0].2.root, expected);
    }

    #[test]
    fn wrong_contents_is_a_mismatch_not_a_match() {
        let root = tempfile::tempdir().unwrap();
        let path = write(root.path(), "ep1.mkv", b"a different encode");
        let expected = ed2k_hash_bytes(b"the real file").root;
        let (resolution, fresh) = resolve_with_cache(
            "ep1.mkv",
            expected,
            &[root.path().to_path_buf()],
            &HashMap::new(),
        );
        assert_eq!(resolution, Resolution::HashMismatch(path));
        // Mismatched contents still got hashed; cache the truth.
        assert_eq!(fresh.len(), 1);
    }

    #[test]
    fn a_verified_copy_beats_an_earlier_mismatch() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"the real file".as_slice();
        write(root.path(), "a/ep1.mkv", b"a different encode");
        let good = write(root.path(), "z/ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;
        let (resolution, _) = resolve_with_cache(
            "ep1.mkv",
            expected,
            &[root.path().to_path_buf()],
            &HashMap::new(),
        );
        assert_eq!(resolution, Resolution::Verified(good));
    }

    #[test]
    fn absent_file_is_not_found_and_name_match_is_exact() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "other.mkv", b"x");
        write(root.path(), "ep1.mkv.part", b"x");
        write(root.path(), "xep1.mkv", b"x");
        let (resolution, fresh) = resolve_with_cache(
            "ep1.mkv",
            hash(0),
            &[root.path().to_path_buf()],
            &HashMap::new(),
        );
        assert_eq!(resolution, Resolution::NotFound);
        assert!(fresh.is_empty());
    }

    /// The cache is believed when (mtime, size) match — proven by
    /// poisoning it: a cache row claiming the *wrong* root makes the
    /// resolver miss a file whose real contents match.
    #[test]
    fn matching_mtime_and_size_trusts_the_cache_without_rehashing() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        let path = write(root.path(), "ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;
        let metadata = std::fs::metadata(&path).unwrap();
        let mtime = mtime_millis(&metadata).unwrap();

        let mut poisoned = ed2k_hash_bytes(b"something else entirely");
        poisoned.size_bytes = contents.len() as u64;
        let cache = HashMap::from([(path.clone(), (mtime, poisoned))]);

        let (resolution, fresh) =
            resolve_with_cache("ep1.mkv", expected, &[root.path().to_path_buf()], &cache);
        // The poisoned root doesn't match and wasn't re-read: mismatch.
        assert_eq!(resolution, Resolution::HashMismatch(path));
        assert!(fresh.is_empty(), "a cache hit must not re-hash");
    }

    /// A stale mtime invalidates the cache row: the file is re-hashed
    /// (and the fresh hash reported), so a touched file recovers.
    #[test]
    fn stale_mtime_rehashes() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        let path = write(root.path(), "ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;

        let mut poisoned = ed2k_hash_bytes(b"something else entirely");
        poisoned.size_bytes = contents.len() as u64;
        // Same size, *different* mtime: stale.
        let cache = HashMap::from([(path.clone(), (-1, poisoned))]);

        let (resolution, fresh) =
            resolve_with_cache("ep1.mkv", expected, &[root.path().to_path_buf()], &cache);
        assert_eq!(resolution, Resolution::Verified(path));
        assert_eq!(fresh.len(), 1, "the re-hash must be reported for caching");
    }

    // ---- the eviction rule.

    fn cache_entry(i: u8, last_access: i64) -> CacheEntry {
        CacheEntry {
            hash: hash(i),
            path: PathBuf::from(format!("/cache/{i}.mkv")),
            size_bytes: 1_000,
            last_access,
        }
    }

    #[test]
    fn eviction_rule_boundaries() {
        let week = Duration::from_secs(7 * 24 * 3600);
        let entry = cache_entry(1, 1_000);
        let week_millis = week.as_millis() as i64;

        // Unwatched or protected: never, under any retention.
        for retention in [
            CacheRetention::AfterWatch,
            CacheRetention::Keep(week),
            CacheRetention::Infinite,
        ] {
            assert!(!evictable(i64::MAX, retention, &entry, false, false));
            assert!(!evictable(i64::MAX, retention, &entry, true, true));
        }

        // AfterWatch: gone at the next pass.
        assert!(evictable(1_000, CacheRetention::AfterWatch, &entry, true, false));

        // Keep(week): exact boundary is evictable, one millisecond
        // before is not.
        assert!(!evictable(
            1_000 + week_millis - 1,
            CacheRetention::Keep(week),
            &entry,
            true,
            false
        ));
        assert!(evictable(
            1_000 + week_millis,
            CacheRetention::Keep(week),
            &entry,
            true,
            false
        ));

        // Infinite: never.
        assert!(!evictable(
            i64::MAX,
            CacheRetention::Infinite,
            &entry,
            true,
            false
        ));
    }

    // ---- actor-level tests (real fs, in-memory storage).

    fn test_clock() -> Clock {
        Arc::new(|| 1_700_000_000_000)
    }

    struct Rig {
        commands: mpsc::Sender<FileCommand>,
        outputs: mpsc::Receiver<FileOutput>,
        _cache_dir: tempfile::TempDir,
    }

    fn spawn_rig(storage: Storage, roots: Vec<PathBuf>, retention: CacheRetention) -> Rig {
        let cache_dir = tempfile::tempdir().unwrap();
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (out_tx, out_rx) = mpsc::channel(64);
        tokio::spawn(run(
            FileConfig {
                storage,
                media_roots: roots,
                retention,
                cache_dir: cache_dir.path().to_path_buf(),
                clock: test_clock(),
            },
            cmd_rx,
            out_tx,
        ));
        Rig {
            commands: cmd_tx,
            outputs: out_rx,
            _cache_dir: cache_dir,
        }
    }

    async fn next_output(rig: &mut Rig) -> FileOutput {
        tokio::time::timeout(Duration::from_secs(10), rig.outputs.recv())
            .await
            .expect("output timeout")
            .expect("actor gone")
    }

    #[tokio::test]
    async fn resolve_commits_hashes_and_second_resolve_uses_them() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        write(root.path(), "ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );

        rig.commands
            .send(FileCommand::Resolve {
                file: expected,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { resolution, .. } => {
                assert!(matches!(resolution, Resolution::Verified(_)));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        // The hash was persisted: a fresh connection sees it.
        let check = Storage::open(&db_path).unwrap();
        let rows = check.hash_cache().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hash.root, expected);
    }

    #[tokio::test]
    async fn hash_add_serves_cache_hits_without_progress() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        let path = write(root.path(), "ep1.mkv", contents);
        let hashed = ed2k_hash_bytes(contents);

        let storage = Storage::open_in_memory().unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        storage
            .upsert_hash_cache(&path, mtime_millis(&metadata).unwrap(), &hashed, 1)
            .unwrap();

        let mut rig = spawn_rig(storage, vec![], CacheRetention::default());
        rig.commands
            .send(FileCommand::HashAdd {
                path: path.clone(),
                after: None,
            })
            .await
            .unwrap();
        // Straight to Done — no Progress event, no re-read.
        match next_output(&mut rig).await {
            FileOutput::Hash(HashEvent::Done(done)) => {
                assert_eq!(done.result.unwrap(), hashed);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manual_mapping_resolves_verified_and_persists() {
        let root = tempfile::tempdir().unwrap();
        let path = write(root.path(), "differently-named.mkv", b"whatever");

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let mut rig = spawn_rig(
            Storage::open(&db_path).unwrap(),
            vec![],
            CacheRetention::default(),
        );
        rig.commands
            .send(FileCommand::SetManualMapping {
                file: hash(1),
                path: path.clone(),
                series: Some(SeriesKey::Name("Frieren".into())),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { file, resolution } => {
                assert_eq!(file, hash(1));
                assert_eq!(resolution, Resolution::Verified(path.clone()));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        let check = Storage::open(&db_path).unwrap();
        assert_eq!(check.manual_mapping(hash(1)).unwrap().unwrap(), path);
        assert_eq!(
            check
                .series_map_dir(&SeriesKey::Name("Frieren".into()))
                .unwrap()
                .unwrap(),
            path.parent().unwrap()
        );
    }

    #[tokio::test]
    async fn series_known_consults_watch_history() {
        let storage = Storage::open_in_memory().unwrap();
        storage
            .record_watched(&WatchRecord {
                hash: hash(9),
                series_id: Some(AniDbSeriesId(5)),
                series_name: Some("Frieren".into()),
                filename: "frieren-01.mkv".into(),
                watched_at: 1,
            })
            .unwrap();
        let mut rig = spawn_rig(storage, vec![], CacheRetention::default());

        rig.commands
            .send(FileCommand::CheckSeriesKnown {
                file: hash(1),
                series: Some(AniDbSeriesId(5)),
                key: SeriesKey::AniDb(AniDbSeriesId(5)),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::SeriesKnown { known, .. } => assert!(known),
            other => panic!("unexpected output: {other:?}"),
        }

        rig.commands
            .send(FileCommand::CheckSeriesKnown {
                file: hash(2),
                series: None,
                key: SeriesKey::Name("Unknown Show".into()),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::SeriesKnown { known, .. } => assert!(!known),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn archive_moves_the_file_and_rekeys_bookkeeping() {
        let cache = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let contents = b"a cached episode".as_slice();
        let cached_path = write(cache.path(), "ep1.mkv", contents);
        let hashed = ed2k_hash_bytes(contents);

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hashed.root,
                path: cached_path.clone(),
                size_bytes: contents.len() as u64,
                last_access: 1,
            })
            .unwrap();
        let metadata = std::fs::metadata(&cached_path).unwrap();
        storage
            .upsert_hash_cache(&cached_path, mtime_millis(&metadata).unwrap(), &hashed, 1)
            .unwrap();

        let dest = library.path().join("Frieren/Season 1/ep1.mkv");
        let mut rig = spawn_rig(storage, vec![], CacheRetention::default());
        rig.commands
            .send(FileCommand::Archive {
                file: hashed.root,
                dest: dest.clone(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Archived { result, .. } => assert_eq!(result.unwrap(), dest),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(dest.is_file());
        assert!(!cached_path.exists());

        let check = Storage::open(&db_path).unwrap();
        assert!(check.cache_entries().unwrap().is_empty());
        let rows = check.hash_cache().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, dest);

        // Archiving something not in the cache fails politely.
        rig.commands
            .send(FileCommand::Archive {
                file: hash(7),
                dest: library.path().join("nope.mkv"),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Archived { result, .. } => assert!(result.is_err()),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn eviction_deletes_watched_unprotected_files_only() {
        let cache = tempfile::tempdir().unwrap();
        let watched_path = write(cache.path(), "watched.mkv", b"watched");
        let protected_path = write(cache.path(), "protected.mkv", b"protected");
        let unwatched_path = write(cache.path(), "unwatched.mkv", b"unwatched");

        let storage = Storage::open_in_memory().unwrap();
        for (i, path) in [
            (1u8, &watched_path),
            (2, &protected_path),
            (3, &unwatched_path),
        ] {
            storage
                .upsert_cache_entry(&CacheEntry {
                    hash: hash(i),
                    path: path.clone(),
                    size_bytes: 10,
                    last_access: 0,
                })
                .unwrap();
        }
        // hash(1) personally watched; hash(2) watched but protected.
        for i in [1u8, 2] {
            storage
                .record_watched(&WatchRecord {
                    hash: hash(i),
                    series_id: None,
                    series_name: None,
                    filename: format!("{i}.mkv"),
                    watched_at: 1,
                })
                .unwrap();
        }

        let mut rig = spawn_rig(storage, vec![], CacheRetention::AfterWatch);
        rig.commands
            .send(FileCommand::RunEviction {
                protected: HashSet::from([hash(2)]),
                group_watched: HashSet::new(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Evicted { files } => assert_eq!(files, vec![hash(1)]),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(!watched_path.exists());
        assert!(protected_path.exists());
        assert!(unwatched_path.exists());
    }

    #[tokio::test]
    async fn group_watched_flag_makes_behind_the_group_evictable() {
        let cache = tempfile::tempdir().unwrap();
        let behind_path = write(cache.path(), "behind.mkv", b"behind");
        let storage = Storage::open_in_memory().unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hash(1),
                path: behind_path.clone(),
                size_bytes: 10,
                last_access: 0,
            })
            .unwrap();
        // Never personally watched — but the group moved past it.
        let mut rig = spawn_rig(storage, vec![], CacheRetention::AfterWatch);
        rig.commands
            .send(FileCommand::RunEviction {
                protected: HashSet::new(),
                group_watched: HashSet::from([hash(1)]),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Evicted { files } => assert_eq!(files, vec![hash(1)]),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(!behind_path.exists());
    }
}
