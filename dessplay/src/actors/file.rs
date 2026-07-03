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
use dessplay_core::net::{PeerId, PeerMessage, chunk_range};
use dessplay_core::types::{AniDbSeriesId, Ed2kHash, FileAvailability};
use tokio::sync::mpsc;

use crate::actors::network::Clock;
use crate::config::CacheRetention;
use crate::download::{DownloadAction, DownloadConfig, Downloads};
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
    /// Move a cached download into the library under the download root
    /// (the first media root). The actor builds the destination —
    /// `<download root>/<series>/<filename>` — since it owns the media
    /// roots (design.md, Archive).
    Archive {
        /// The cached file.
        file: Ed2kHash,
        /// Series name for the subdirectory (synced metadata always has
        /// one); `None` falls back to an "Unsorted" folder.
        series_name: Option<String>,
        /// Original filename to archive under.
        filename: String,
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
    /// Scan the media library now (new/changed files → hashes → index).
    /// Fired internally at startup and on a timer; also exposed for a
    /// manual refresh and for tests.
    RescanLibrary,

    // ---- File transfer (Phase 9B).
    /// Begin (or refresh) downloading `file` from `sources`. Idempotent;
    /// the session re-issues it as peers/availability/playback change.
    StartDownload {
        /// The file (its root is the playlist key / id).
        file: Ed2kHash,
        /// File size, for chunk/block geometry.
        size_bytes: u64,
        /// Candidate source peers (present peers that have it).
        sources: Vec<dessplay_core::net::PeerId>,
        /// Playback chunk anchor for the sequential window.
        play_chunk: u32,
    },
    /// A file-transfer message relayed from a peer (download or serve).
    PeerMessage {
        /// The sender.
        from: dessplay_core::net::PeerId,
        /// The message.
        message: Box<dessplay_core::net::PeerMessage>,
    },
    /// A local copy vanished under us mid-session (the player failed to
    /// load it). Drop it from the servable set, prune its cache/hash
    /// bookkeeping, and flip availability to Missing so it re-resolves —
    /// the same "drop + prune + re-resolve" guard the serve path runs
    /// (design.md, Download Cache: two runtime guards).
    ForgetLocalFile {
        /// The file whose local copy is gone.
        file: Ed2kHash,
    },
}

/// A file the library scan has identified. The mtime rides to the server's
/// lookup queue so re-validation backoff reflects the file's real age; the
/// `series_hint` (a title-like ancestor directory name) rides along so the
/// server can group AniDB-unknown episodes under their series folder rather
/// than the per-episode filename stem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedFile {
    /// The file's ed2k root hash.
    pub hash: Ed2kHash,
    /// File size in bytes.
    pub size: u64,
    /// Display/match filename (the path's final component).
    pub filename: String,
    /// Modification time in unix millis.
    pub mtime: i64,
    /// A title-like containing-directory name, or `None` if none looked like
    /// a title (see [`dir_series_hint`]).
    pub series_hint: Option<String>,
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
    /// A watch record was just written to local watch history. Carries no
    /// payload: it exists only to prompt a fresh UI snapshot, so the
    /// Recent Series pane (which reads `watch_history` at snapshot time)
    /// reflects the new recency immediately. Watch recording produces no
    /// sync event, so without this the pane wouldn't update until an
    /// unrelated network event happened along.
    WatchRecorded,
    /// A batch of indexed library files (hash, size, filename), from the
    /// media-library scan. The session inserts AniDB lookup requests for
    /// any that still lack metadata. Emitted incrementally: cache hits
    /// up front, then one per file as it finishes hashing.
    LibraryIndexed {
        /// Newly-known files from the media-library scan.
        files: Vec<IndexedFile>,
    },
    /// Library-scan hashing progress (the no-silent-work rule). Emitted
    /// while files are being hashed; `done == total` marks the end.
    ScanProgress {
        /// Files hashed so far this scan.
        done: usize,
        /// Files needing a hash this scan.
        total: usize,
    },

    // ---- File transfer (Phase 9B).
    /// Relay a file-transfer message to a peer (chunk request/data,
    /// block hashes, availability, cancel). The bridge loop turns this
    /// into a `NetworkCommand::SendPeer`.
    SendPeer {
        /// Destination peer.
        to: dessplay_core::net::PeerId,
        /// The message.
        message: Box<dessplay_core::net::PeerMessage>,
    },
    /// Our availability for a file changed (download progress / Ready).
    /// The bridge loop writes it to the synced `FileAvailability`.
    Availability {
        /// The file.
        file: Ed2kHash,
        /// New availability.
        availability: FileAvailability,
    },
    /// A download finished and verified at `path` (now a local copy).
    DownloadComplete {
        /// The file.
        file: Ed2kHash,
        /// The complete file in the cache.
        path: PathBuf,
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
    /// Download scheduler tuning (pipeline depth etc.).
    pub download: DownloadConfig,
    /// Upload rate cap, bytes/sec (`None` = unlimited).
    pub upload_limit: Option<u64>,
    /// How often to scan the media library for new/changed files (and
    /// at startup). `None` disables scanning entirely (tests). Interactive
    /// clients use ~60s; a seeder, whose store is large and stable, ~24h.
    pub scan_interval: Option<std::time::Duration>,
    /// How long after the last transfer traffic (serving or downloading)
    /// scan *hashing* stays deferred (#21). Indexing is bulk disk work
    /// with no deadline; transfers are latency-sensitive (a silent
    /// source is snubbed at 30s), so hashing yields and resumes once
    /// transfers go quiet. Use [`SCAN_TRANSFER_QUIET_DEFAULT`].
    pub scan_transfer_quiet: std::time::Duration,
}

/// Production default for [`FileConfig::scan_transfer_quiet`].
pub const SCAN_TRANSFER_QUIET_DEFAULT: std::time::Duration = std::time::Duration::from_secs(10);

/// Completions from blocking subtasks.
enum Done {
    Resolved {
        file: Ed2kHash,
        /// The filename that was searched for — kept so a mismatch can
        /// be re-resolved later without the session re-asking (#26).
        filename: String,
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
    /// A media-library walk finished: files already in the hash cache
    /// (known hashes) plus a worklist of files needing a hash.
    LibraryWalk {
        /// Cache hits — known immediately.
        hits: Vec<IndexedFile>,
        /// Files to hash (new or changed since last scan).
        worklist: std::collections::VecDeque<ScanItem>,
        /// Index rows whose files vanished from under the roots (moved
        /// or deleted behind the app's back) — to be pruned.
        stale: Vec<PathBuf>,
    },
    /// One library-scan file finished hashing.
    LibraryHashed {
        /// The file that was hashed.
        item: ScanItem,
        /// Its hash, or why not.
        result: std::io::Result<Ed2kFileHash>,
    },
    /// A manually-mapped file finished hashing (so we can serve it).
    ManualHashed {
        /// The playlist hash the mapping is for.
        file: Ed2kHash,
        /// The mapped path that was hashed.
        path: PathBuf,
        /// Its mtime at hash time (cache validity key).
        mtime: Option<i64>,
        /// Its hash, or why not.
        result: std::io::Result<Ed2kFileHash>,
    },
}

/// A media-root file the scan needs to hash (cache miss or changed).
#[derive(Clone, Debug)]
pub struct ScanItem {
    /// Absolute path.
    path: PathBuf,
    /// Modification time in unix millis (hash-cache validity key).
    mtime: i64,
    /// Display/match filename (the path's final component).
    filename: String,
    /// A title-like containing-directory name (see [`dir_series_hint`]).
    series_hint: Option<String>,
}

/// Run the actor until the command channel closes.
pub async fn run(
    config: FileConfig,
    mut commands: mpsc::Receiver<FileCommand>,
    out: mpsc::Sender<FileOutput>,
) {
    let (done_tx, mut done_rx) = mpsc::channel::<Done>(64);
    // Captured before `config` moves into the actor.
    let scan_interval = config.scan_interval;
    let scan_enabled = scan_interval.is_some();
    let mut actor = match Actor::new(config, out, done_tx) {
        Ok(actor) => actor,
        Err(e) => {
            tracing::error!("file actor failed to initialize: {e}");
            return;
        }
    };
    // Drives snub detection, pipeline refill, and serve-queue draining.
    let mut tick = tokio::time::interval(DOWNLOAD_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Media-library scan: the first tick fires immediately (startup scan),
    // then every `scan_interval`. Disabled (guarded off) when `None`.
    let mut scan_tick =
        tokio::time::interval(scan_interval.unwrap_or(std::time::Duration::from_secs(3600)));
    scan_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
            _ = tick.tick() => {
                actor.on_tick().await;
            }
            _ = scan_tick.tick(), if scan_enabled => {
                actor.start_library_scan();
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
    /// Local complete copies we can serve from (hash → path): verified
    /// resolutions, manual mappings, and completed downloads.
    local_files: HashMap<Ed2kHash, PathBuf>,
    /// Active downloads (the scheduling brain).
    downloads: Downloads,
    /// Pending chunks to serve to peers: (requester, file, chunk).
    serve_queue: std::collections::VecDeque<(PeerId, Ed2kHash, u32)>,
    /// Upload pacing for serving chunks (`None` = unlimited).
    upload: UploadLimiter,
    /// Last shared-clock millis we wrote a `Downloading` progress
    /// update, per file (≤1/s throttle).
    last_progress_at: HashMap<Ed2kHash, u64>,
    /// Library-scan files still awaiting a hash (FIFO, one at a time).
    scan_worklist: std::collections::VecDeque<ScanItem>,
    /// Whether a scan-hash is currently running (caps scan hashing at one
    /// at a time so the initial whole-library hash is a background trickle
    /// that never starves interactive resolves/adds on the blocking pool).
    scan_hashing: bool,
    /// Whether a library walk is queued/running (avoids overlapping walks).
    scan_walking: bool,
    /// Files hashed / total this scan, for `ScanProgress`.
    scan_done: usize,
    scan_total: usize,
    /// When the current scan's hashing started, for the completion summary.
    scan_started: Option<std::time::Instant>,
    /// Hash failures seen this scan, reported in the completion summary.
    scan_failed: usize,
    /// Files between info-level progress checkpoints (~20 over the scan).
    scan_log_step: usize,
    /// Mismatched resolutions being watched for quiescence (#26):
    /// name-matched files whose contents didn't hash — usually a copy or
    /// external download still being written into a media root.
    rechecks: HashMap<Ed2kHash, Recheck>,
    /// When transfer traffic (serving or downloading) last happened;
    /// scan hashing defers within [`FileConfig::scan_transfer_quiet`] of
    /// it (#21).
    last_transfer_activity: Option<std::time::Instant>,
    scan_transfer_quiet: std::time::Duration,
    /// One "deferring" log line per deferral episode, not per tick.
    scan_defer_logged: bool,
    /// Scan hash results not yet folded into `hash_cache` (batched --
    /// see [`SCAN_COMMIT_BATCH`]).
    scan_pending_commits: Vec<(PathBuf, i64, Ed2kFileHash)>,
    out: mpsc::Sender<FileOutput>,
    done_tx: mpsc::Sender<Done>,
}

/// How many library-scan hash results to buffer before folding them into
/// `hash_cache` in a single clone, instead of cloning the whole map once
/// per file. A full scan hashes one file at a time
/// ([`Actor::pump_library_scan`]), so without batching a large library
/// makes the per-file `commit_fresh_hashes` clone O(n) work n times —
/// O(n^2) total, and a burst of ever-larger transient allocations that
/// fragments the allocator (2026-07-03: `malloc_trim` recovered ~360MB
/// RSS on the primary seeder after a scan).
const SCAN_COMMIT_BATCH: usize = 64;

/// One watched mismatch (#26): poll the path's `(mtime, size)` about
/// once a second; once it holds still — and differs from the state the
/// failed hash saw — re-resolve. A stable mismatch (a different encode)
/// never re-hashes: its hash-cache row still matches the disk.
struct Recheck {
    path: PathBuf,
    filename: String,
    /// `(mtime, size)` at the last poll.
    observed: Option<(i64, u64)>,
    /// Consecutive polls with `observed` unchanged.
    quiet_polls: u32,
    last_poll: std::time::Instant,
    /// Watch-episode deadline; after this the entry is dropped (the
    /// periodic library scan remains the long-tail safety net).
    deadline: std::time::Instant,
}

/// Recheck poll cadence (cheap: one `stat` per watched file).
const RECHECK_POLL: std::time::Duration = std::time::Duration::from_secs(1);
/// Consecutive unchanged polls before a changed file counts as quiet.
const RECHECK_QUIET_POLLS: u32 = 2;
/// How long a mismatch stays watched before the watch is dropped.
const RECHECK_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

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
        let mut hash_cache: HashMap<PathBuf, (i64, Ed2kFileHash)> = config
            .storage
            .hash_cache()?
            .into_iter()
            .map(|row| (row.path, (row.mtime, row.hash)))
            .collect();
        let manual: HashMap<Ed2kHash, PathBuf> =
            config.storage.manual_mappings()?.into_iter().collect();
        // Manual mappings are servable local copies too.
        let mut local_files = manual.clone();
        // Reconcile the download cache against the filesystem: the DB is
        // an index, the disk is the truth. A row whose file the user
        // deleted or truncated is pruned (so the entry re-resolves to
        // Missing and re-downloads); a survivor is registered as a
        // servable, hash-addressed copy so restarts re-recognize it
        // (the cache is hash-named and the filename search can't find it).
        let mut reconciled = 0usize;
        let mut pruned = 0usize;
        for entry in config.storage.cache_entries().unwrap_or_default() {
            let live = std::fs::metadata(&entry.path)
                .map(|m| m.len() == entry.size_bytes)
                .unwrap_or(false);
            if live {
                local_files.insert(entry.hash, entry.path);
                reconciled += 1;
            } else {
                tracing::warn!(
                    path = %entry.path.display(),
                    "cached file missing or wrong size; pruning stale bookkeeping"
                );
                if let Err(e) = config.storage.remove_cache_entry(entry.hash) {
                    tracing::error!("pruning cache entry: {e}");
                }
                if let Err(e) = config.storage.remove_hash_cache(&entry.path) {
                    tracing::error!("pruning hash cache: {e}");
                }
                hash_cache.remove(&entry.path);
                pruned += 1;
            }
        }
        tracing::debug!(
            cached_hashes = hash_cache.len(),
            manual_mappings = manual.len(),
            cache_reconciled = reconciled,
            cache_pruned = pruned,
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
            local_files,
            downloads: Downloads::new(config.download),
            serve_queue: std::collections::VecDeque::new(),
            upload: UploadLimiter::new(config.upload_limit),
            last_progress_at: HashMap::new(),
            scan_pending_commits: Vec::new(),
            scan_worklist: std::collections::VecDeque::new(),
            scan_hashing: false,
            scan_walking: false,
            scan_done: 0,
            scan_total: 0,
            scan_started: None,
            scan_failed: 0,
            scan_log_step: 1,
            rechecks: HashMap::new(),
            last_transfer_activity: None,
            scan_transfer_quiet: config.scan_transfer_quiet,
            scan_defer_logged: false,
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
                // Prompt a fresh UI snapshot so Recent Series re-reads
                // watch history and reflects the new recency at once.
                let _ = self.out.send(FileOutput::WatchRecorded).await;
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
            FileCommand::Archive {
                file,
                series_name,
                filename,
            } => self.archive(file, series_name, filename).await,
            FileCommand::RunEviction {
                protected,
                group_watched,
            } => self.run_eviction(&protected, &group_watched).await,
            FileCommand::SetMediaRoots(roots) => {
                self.media_roots = roots;
                // New roots may hold files we've never indexed.
                self.start_library_scan();
            }
            FileCommand::SetRetention(retention) => self.retention = retention,
            FileCommand::RescanLibrary => self.start_library_scan(),
            FileCommand::StartDownload {
                file,
                size_bytes,
                sources,
                play_chunk,
            } => {
                let path = self.download_path(file);
                // The session re-emits StartDownload every snapshot to
                // refresh sources (idempotent); log only the actor-side
                // birth of a download. This should mirror the session's
                // "prefetch: starting download" transition one-for-one —
                // a divergence means the two disagree about what is in
                // flight, which is worth seeing.
                if !self.downloads.is_active(&file) {
                    tracing::info!(%file, sources = sources.len(), "starting download");
                }
                let actions = self.downloads.start(
                    file,
                    size_bytes,
                    file, // root == playlist key
                    path,
                    sources,
                    play_chunk,
                    (self.clock)(),
                );
                self.run_download_actions(actions).await;
            }
            FileCommand::PeerMessage { from, message } => {
                self.on_peer_message(from, *message).await;
            }
            FileCommand::ForgetLocalFile { file } => self.lost_local_file(file).await,
        }
    }

    /// Periodic maintenance: snub/refill the downloads, drain the serve
    /// queue within the upload budget.
    async fn on_tick(&mut self) {
        let actions = self.downloads.tick((self.clock)());
        self.run_download_actions(actions).await;
        self.drain_serve_queue().await;
        self.poll_rechecks().await;
        // Resume deferred scan hashing once transfers go quiet (#21).
        self.pump_library_scan();
    }

    /// Poll watched mismatches (#26): about one `stat` per second per
    /// watched file. A file whose `(mtime, size)` changed since the
    /// failed hash and then held still for a couple of polls is
    /// re-resolved; one that never changes (a genuine different encode)
    /// is never re-hashed and its watch expires.
    async fn poll_rechecks(&mut self) {
        let now = std::time::Instant::now();
        let due: Vec<Ed2kHash> = self
            .rechecks
            .iter()
            .filter(|(_, r)| now.duration_since(r.last_poll) >= RECHECK_POLL)
            .map(|(file, _)| *file)
            .collect();
        for file in due {
            let Some(r) = self.rechecks.get_mut(&file) else {
                continue;
            };
            r.last_poll = now;
            if now >= r.deadline {
                tracing::debug!(%file, path = %r.path.display(),
                    "mismatch watch expired without the file changing");
                self.rechecks.remove(&file);
                continue;
            }
            let stat = std::fs::metadata(&r.path)
                .ok()
                .and_then(|m| Some((mtime_millis(&m)?, m.len())));
            let Some(stat) = stat else {
                // Gone from under us; a later scan or resolve handles it.
                self.rechecks.remove(&file);
                continue;
            };
            if r.observed == Some(stat) {
                r.quiet_polls += 1;
                // The hash-cache row is keyed by the (mtime, size) the
                // failed hash read; matching it means the contents are
                // already known-mismatched — nothing new to check.
                let hashed_state = self
                    .hash_cache
                    .get(&r.path)
                    .map(|(mtime, h)| (*mtime, h.size_bytes));
                if r.quiet_polls >= RECHECK_QUIET_POLLS && hashed_state != Some(stat) {
                    let filename = r.filename.clone();
                    tracing::info!(%file, path = %r.path.display(),
                        "mismatched file quiesced after changing; re-checking");
                    self.rechecks.remove(&file);
                    self.resolve(file, filename).await;
                }
            } else {
                r.observed = Some(stat);
                r.quiet_polls = 0;
            }
        }
    }

    /// Note transfer traffic; scan hashing defers while it's recent.
    fn note_transfer_activity(&mut self) {
        self.last_transfer_activity = Some(std::time::Instant::now());
    }

    /// Whether transfer traffic happened within the deferral window.
    fn transfer_recent(&self) -> bool {
        self.last_transfer_activity
            .is_some_and(|at| at.elapsed() < self.scan_transfer_quiet)
    }

    /// The cache path a download assembles into.
    fn download_path(&self, file: Ed2kHash) -> PathBuf {
        self.cache_dir.join(file.to_string())
    }

    /// Kick off a media-library scan: walk the roots in the background,
    /// turning up new/changed video files to hash. Skips if a walk is
    /// already queued/running (overlapping walks would duplicate work).
    fn start_library_scan(&mut self) {
        if self.scan_walking {
            return;
        }
        self.scan_walking = true;
        let roots = self.media_roots.clone();
        let cache = Arc::clone(&self.hash_cache);
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let (hits, worklist, stale) = scan_library(&roots, &cache);
            tracing::debug!(
                hits = hits.len(),
                to_hash = worklist.len(),
                stale = stale.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "library walk finished"
            );
            let _ = done_tx.blocking_send(Done::LibraryWalk {
                hits,
                worklist,
                stale,
            });
        });
    }

    /// Hash the next worklist file, if any and none is already in flight.
    /// One at a time, so the initial whole-library hash is a background
    /// trickle that never floods the blocking pool. Deferred entirely
    /// while transfer traffic is recent (#21): indexing has no deadline,
    /// but a source slowed to a crawl by scan disk I/O gets snubbed at
    /// 30s and the download stalls. `on_tick` re-pumps, so a deferred
    /// worklist resumes once transfers go quiet.
    fn pump_library_scan(&mut self) {
        if self.scan_hashing {
            return;
        }
        if !self.scan_worklist.is_empty() && self.transfer_recent() {
            if !self.scan_defer_logged {
                self.scan_defer_logged = true;
                tracing::info!(
                    pending = self.scan_worklist.len(),
                    "deferring library hashing while transfers are active"
                );
            }
            return;
        }
        let Some(item) = self.scan_worklist.pop_front() else {
            return;
        };
        self.scan_defer_logged = false;
        self.scan_hashing = true;
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = std::fs::File::open(&item.path).and_then(ed2k_hash_reader);
            let _ = done_tx.blocking_send(Done::LibraryHashed { item, result });
        });
    }

    /// Route an incoming peer message: serve-side requests are answered
    /// from our local copies; the rest feed the download scheduler.
    async fn on_peer_message(&mut self, from: PeerId, message: PeerMessage) {
        // Any peer traffic — requests we serve, chunks we receive — is
        // transfer activity that defers scan hashing (#21).
        self.note_transfer_activity();
        match message {
            PeerMessage::BlockHashRequest { file } => self.serve_block_hashes(from, file).await,
            PeerMessage::ChunkRequest { file, chunks } => {
                self.enqueue_serve(from, file, chunks);
                self.drain_serve_queue().await;
            }
            PeerMessage::Cancel { file, chunks } => {
                let cancelled: HashSet<u32> = chunks.into_iter().collect();
                self.serve_queue
                    .retain(|job| !(job.0 == from && job.1 == file && cancelled.contains(&job.2)));
            }
            download_msg => {
                let actions = self
                    .downloads
                    .on_peer_message(from, download_msg, (self.clock)());
                self.run_download_actions(actions).await;
            }
        }
    }

    /// Apply the scheduler's actions: relay messages, write progress
    /// (≤1/s), and record completions as servable local copies.
    async fn run_download_actions(&mut self, actions: Vec<DownloadAction>) {
        if !actions.is_empty() {
            self.note_transfer_activity();
        }
        for action in actions {
            match action {
                DownloadAction::Send { to, message } => {
                    // The outgoing edge of every download request funnels
                    // through here, so log it here -- a stalled download is
                    // otherwise a black box (we saw `starting download` then
                    // nothing). Debug, not trace: trace is off in most
                    // sessions and a stall is exactly when the log is read.
                    // Volume is low -- block hashes once per (re)ask, chunk
                    // requests batched -- so this never floods even a fast
                    // download. Cancels are endgame noise, kept at trace.
                    match &message {
                        PeerMessage::BlockHashRequest { file } => {
                            tracing::debug!(%file, %to, "requesting block hashes")
                        }
                        PeerMessage::ChunkRequest { file, chunks } => {
                            tracing::debug!(%file, %to, count = chunks.len(), "requesting chunks")
                        }
                        PeerMessage::Cancel { file, chunks } => {
                            tracing::trace!(%file, %to, count = chunks.len(), "cancelling chunks")
                        }
                        _ => {}
                    }
                    let _ = self
                        .out
                        .send(FileOutput::SendPeer {
                            to,
                            message: Box::new(message),
                        })
                        .await;
                }
                DownloadAction::Progress { file, progress_bps } => {
                    // Throttle progress writes to at most once a second
                    // (a fast download crosses a block — a progress
                    // step — several times a second).
                    let now = (self.clock)();
                    if now.saturating_sub(*self.last_progress_at.get(&file).unwrap_or(&0)) >= 1000 {
                        self.last_progress_at.insert(file, now);
                        let _ = self
                            .out
                            .send(FileOutput::Availability {
                                file,
                                availability: FileAvailability::Downloading { progress_bps },
                            })
                            .await;
                    }
                }
                DownloadAction::Complete {
                    file,
                    path,
                    block_hashes,
                } => self.on_download_complete(file, path, block_hashes).await,
                DownloadAction::Abandon { file, reason } => {
                    tracing::warn!(%file, "download abandoned: {reason}");
                    let _ = self
                        .out
                        .send(FileOutput::Availability {
                            file,
                            availability: FileAvailability::Missing,
                        })
                        .await;
                }
            }
        }
    }

    /// A download finished: record it as a servable local copy, cache
    /// its block hashes (so we can re-serve them) and cache bookkeeping,
    /// and surface Ready + DownloadComplete.
    async fn on_download_complete(
        &mut self,
        file: Ed2kHash,
        path: PathBuf,
        block_hashes: Vec<dessplay_core::hash::Ed2kBlockHash>,
    ) {
        tracing::info!(%file, path = %path.display(), "download complete");
        self.local_files.insert(file, path.clone());
        self.last_progress_at.remove(&file);
        if let Ok(metadata) = std::fs::metadata(&path) {
            let size = metadata.len();
            let now = (self.clock)() as i64;
            if let Err(e) = self.storage.upsert_cache_entry(&CacheEntry {
                hash: file,
                path: path.clone(),
                size_bytes: size,
                last_access: now,
            }) {
                tracing::error!("cache bookkeeping after download: {e}");
            }
            // Cache the validated block hashes so we can serve them on.
            if let Some(mtime) = mtime_millis(&metadata) {
                let hashed = Ed2kFileHash {
                    root: file,
                    blocks: block_hashes,
                    size_bytes: size,
                };
                self.commit_fresh_hashes(vec![(path.clone(), mtime, hashed)]);
            }
        }
        let _ = self
            .out
            .send(FileOutput::Availability {
                file,
                availability: FileAvailability::Ready,
            })
            .await;
        let _ = self
            .out
            .send(FileOutput::DownloadComplete { file, path })
            .await;
    }

    /// Serve a peer the per-block hashes of a file we hold (from the
    /// hash cache). Silently ignored if we don't have the file or its
    /// hashes cached.
    async fn serve_block_hashes(&mut self, to: PeerId, file: Ed2kHash) {
        let Some(path) = self.local_files.get(&file).cloned() else {
            // A peer asked us for this file's block hashes -- so it picked
            // us as a source, meaning we advertised it Ready -- but we no
            // longer hold it. This silent bail is a prime suspect for a
            // downloader stuck waiting on block hashes that never come.
            tracing::debug!(%file, %to, "asked for block hashes we don't hold; ignoring");
            return;
        };
        // Don't advertise a file the user deleted under us: drop it and
        // flip our own availability to Missing (which re-resolves).
        if !path.exists() {
            self.lost_local_file(file).await;
            return;
        }
        let Some((_, hashed)) = self.hash_cache.get(&path) else {
            // We have the file but not its block hashes cached; skip
            // (a re-hash-on-demand path is possible future work). A manual
            // mapping populates this via hash_manual_mapping.
            tracing::debug!(%file, "asked for block hashes we haven't cached");
            return;
        };
        if hashed.root != file {
            // The path we hold hashes to something other than what was
            // requested (e.g. a manual mapping to a different encode). Never
            // serve those block hashes under this file's identity.
            tracing::debug!(%file, actual = %hashed.root,
                "cached hashes don't match the requested file; not serving");
            return;
        }
        let blocks = hashed.blocks.clone();
        let size = hashed.size_bytes;
        tracing::debug!(%file, %to, blocks = blocks.len(), "serving block hashes");
        let _ = self
            .out
            .send(FileOutput::SendPeer {
                to: to.clone(),
                message: Box::new(PeerMessage::BlockHashes {
                    file,
                    hashes: blocks,
                }),
            })
            .await;
        // Bootstrap the downloader's source bitfield: we hold the whole
        // file, so advertise a complete bitfield. (BlockHashRequest is
        // only sent by downloaders, so this can't loop.)
        let mut bitfield = dessplay_core::net::Bitfield::new(dessplay_core::net::chunk_count(size));
        for i in 0..bitfield.len() {
            bitfield.set(i);
        }
        let _ = self
            .out
            .send(FileOutput::SendPeer {
                to,
                message: Box::new(PeerMessage::FileAvailability { file, bitfield }),
            })
            .await;
    }

    /// A local copy we believed we held has vanished from disk. Drop it
    /// from the servable set, prune any cache bookkeeping (a no-op for
    /// media-root files), and flip our own availability to Missing so the
    /// entry re-resolves (and re-downloads if enabled). The disk is the
    /// truth; the DB follows it.
    async fn lost_local_file(&mut self, file: Ed2kHash) {
        if let Some(path) = self.local_files.remove(&file) {
            tracing::warn!(path = %path.display(), %file, "local copy vanished; dropping");
            if let Err(e) = self.storage.remove_cache_entry(file) {
                tracing::error!("pruning cache entry: {e}");
            }
            if let Err(e) = self.storage.remove_hash_cache(&path) {
                tracing::error!("pruning hash cache: {e}");
            }
            let mut cache = (*self.hash_cache).clone();
            cache.remove(&path);
            self.hash_cache = Arc::new(cache);
        }
        let _ = self
            .out
            .send(FileOutput::Availability {
                file,
                availability: FileAvailability::Missing,
            })
            .await;
    }

    /// Queue a peer's chunk requests for serving (deduping re-requests).
    fn enqueue_serve(&mut self, to: PeerId, file: Ed2kHash, chunks: Vec<u32>) {
        if !self.local_files.contains_key(&file) {
            // We advertised Ready but no longer hold it -- same silent
            // "advertised but can't serve" failure as serve_block_hashes,
            // for chunks rather than block hashes.
            tracing::debug!(%file, %to, count = chunks.len(),
                "asked for chunks we don't hold; ignoring");
            return;
        }
        let requested = chunks.len();
        for chunk in chunks {
            let job = (to.clone(), file, chunk);
            if !self.serve_queue.contains(&job) {
                self.serve_queue.push_back(job);
            }
        }
        tracing::debug!(%file, %to, requested, queued = self.serve_queue.len(),
            "queued chunks to serve");
    }

    /// Send queued chunks within the upload budget. Reads are small
    /// (250 KiB) and done inline; the budget paces how many go per tick.
    async fn drain_serve_queue(&mut self) {
        let now = (self.clock)();
        while let Some((to, file, chunk)) = self.serve_queue.front().cloned() {
            let Some(path) = self.local_files.get(&file).cloned() else {
                self.serve_queue.pop_front();
                continue;
            };
            if !path.exists() {
                // Deleted under us: stop serving it and re-resolve.
                self.serve_queue.retain(|job| job.1 != file);
                self.lost_local_file(file).await;
                continue;
            }
            let range = chunk_range(chunk, self.file_size(&path));
            let len = range.end - range.start;
            if !self.upload.try_take(len, now) {
                break; // out of budget this tick
            }
            self.serve_queue.pop_front();
            match read_range(&path, range) {
                Ok(data) => {
                    let _ = self
                        .out
                        .send(FileOutput::SendPeer {
                            to,
                            message: Box::new(PeerMessage::ChunkData {
                                file,
                                index: chunk,
                                data,
                            }),
                        })
                        .await;
                }
                Err(e) => tracing::debug!(%file, chunk, "serving chunk failed: {e}"),
            }
        }
    }

    /// Size of a local file (0 if unstattable — the chunk-range math
    /// then yields an empty range and the read is skipped).
    fn file_size(&self, path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    async fn on_done(&mut self, done: Done) {
        match done {
            Done::Resolved {
                file,
                filename,
                resolution,
                fresh,
            } => {
                self.commit_fresh_hashes(fresh);
                // A verified local copy can be served to peers.
                if let Resolution::Verified(path) = &resolution {
                    self.local_files.insert(file, path.clone());
                    // Resolving (loading) a file is an "access": bump the
                    // cache entry's last_access so retention runs from last
                    // access, not download time (design.md Download Cache).
                    // A no-op for media-root files, which have no cache row.
                    let now = (self.clock)() as i64;
                    if let Err(e) = self.storage.touch_cache_entry(file, now) {
                        tracing::warn!("touch cache entry on resolve: {e}");
                    }
                }
                // A mismatch is watched for quiescence (#26): the usual
                // cause is a copy/external download still being written,
                // which verifies a few seconds after it finishes. Any
                // other outcome ends the watch.
                match &resolution {
                    Resolution::HashMismatch(path) => {
                        let now = std::time::Instant::now();
                        let entry = self.rechecks.entry(file).or_insert_with(|| Recheck {
                            path: path.clone(),
                            filename: filename.clone(),
                            observed: None,
                            quiet_polls: 0,
                            last_poll: now,
                            deadline: now + RECHECK_WINDOW,
                        });
                        // A re-resolve that still mismatches keeps its
                        // original episode deadline (no immortal watch).
                        entry.path = path.clone();
                        entry.filename = filename;
                        entry.quiet_polls = 0;
                    }
                    Resolution::Verified(_) | Resolution::NotFound => {
                        self.rechecks.remove(&file);
                    }
                }
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
            Done::ManualHashed {
                file,
                path,
                mtime,
                result,
            } => match (result, mtime) {
                (Ok(hashed), Some(mtime)) if hashed.root == file => {
                    // Content matches the mapped hash: cache it so we can
                    // serve block hashes to peers.
                    self.commit_fresh_hashes(vec![(path, mtime, hashed)]);
                }
                (Ok(hashed), _) => {
                    // The user mapped a file whose content differs from the
                    // playlist entry (a different encode). Fine for their own
                    // playback (filename-trusted), but we must not serve it to
                    // peers under this file's identity, so we don't cache it.
                    tracing::info!(
                        path = %path.display(),
                        mapped = %file,
                        actual = %hashed.root,
                        "manual mapping content differs from the entry; won't serve to peers"
                    );
                }
                (Err(e), _) => {
                    tracing::debug!(path = %path.display(), "hashing manual mapping failed: {e}");
                }
            },
            Done::Placeholder { file, result } => match result {
                Ok(path) => {
                    let _ = self
                        .out
                        .send(FileOutput::PlaceholderReady { file, path })
                        .await;
                }
                Err(e) => tracing::error!("placeholder render failed: {e}"),
            },
            Done::LibraryWalk {
                hits,
                worklist,
                stale,
            } => {
                self.scan_walking = false;
                self.prune_stale_index(stale);
                // Cache hits are known immediately.
                if !hits.is_empty() {
                    let _ = self
                        .out
                        .send(FileOutput::LibraryIndexed { files: hits })
                        .await;
                }
                self.scan_worklist = worklist;
                self.scan_total = self.scan_worklist.len();
                self.scan_done = 0;
                if self.scan_total > 0 {
                    tracing::info!(to_hash = self.scan_total, "indexing media library");
                    self.scan_started = Some(std::time::Instant::now());
                    self.scan_failed = 0;
                    // ~20 info-level checkpoints regardless of library size;
                    // small libraries log every file.
                    self.scan_log_step = (self.scan_total / 20).max(1);
                    let _ = self
                        .out
                        .send(FileOutput::ScanProgress {
                            done: 0,
                            total: self.scan_total,
                        })
                        .await;
                    self.pump_library_scan();
                }
            }
            Done::LibraryHashed { item, result } => {
                self.scan_hashing = false;
                self.scan_done += 1;
                match result {
                    Ok(hashed) => {
                        let root = hashed.root;
                        let size = hashed.size_bytes;
                        self.queue_scan_hash_commit(item.path.clone(), item.mtime, hashed);
                        tracing::trace!(
                            done = self.scan_done,
                            total = self.scan_total,
                            path = %item.path.display(),
                            "hashed library file"
                        );
                        let _ = self
                            .out
                            .send(FileOutput::LibraryIndexed {
                                files: vec![IndexedFile {
                                    hash: root,
                                    size,
                                    filename: item.filename,
                                    mtime: item.mtime,
                                    series_hint: item.series_hint,
                                }],
                            })
                            .await;
                    }
                    Err(e) => {
                        self.scan_failed += 1;
                        tracing::debug!(path = %item.path.display(), "library scan hash failed: {e}");
                    }
                }
                // Operator-visible progress for the headless seeder (info, so
                // it shows without RUST_LOG): a periodic checkpoint plus a
                // completion summary with timing and failure count.
                if self.scan_done == self.scan_total {
                    // Fold any still-buffered results now, so a resolve
                    // immediately after "scan complete" sees them instead
                    // of waiting on a stray partial batch.
                    self.flush_scan_hash_commits();
                    tracing::info!(
                        hashed = self.scan_done - self.scan_failed,
                        failed = self.scan_failed,
                        elapsed_ms = self
                            .scan_started
                            .map(|t| t.elapsed().as_millis() as u64)
                            .unwrap_or(0),
                        "media library scan complete"
                    );
                    self.scan_started = None;
                } else if self.scan_done.is_multiple_of(self.scan_log_step) {
                    tracing::info!(
                        done = self.scan_done,
                        total = self.scan_total,
                        percent = self.scan_done * 100 / self.scan_total,
                        "indexing media library"
                    );
                }
                let _ = self
                    .out
                    .send(FileOutput::ScanProgress {
                        done: self.scan_done,
                        total: self.scan_total,
                    })
                    .await;
                self.pump_library_scan();
            }
        }
    }

    /// Drop index rows whose files vanished from under the media roots —
    /// the scan's counterpart to [`Self::lost_local_file`]. Without this
    /// a file moved between directories kept its old row forever,
    /// polluting everything built on the index (browser search ghosts,
    /// add-browser anchoring in the wrong directory — 2026-07-02).
    fn prune_stale_index(&mut self, stale: Vec<PathBuf>) {
        if stale.is_empty() {
            return;
        }
        let mut cache = (*self.hash_cache).clone();
        for path in stale {
            tracing::info!(path = %path.display(), "index row for a vanished file; pruning");
            if let Err(e) = self.storage.remove_hash_cache(&path) {
                tracing::error!("pruning hash cache: {e}");
            }
            if let Some((_, hashed)) = cache.remove(&path) {
                // The servable set may still point at the vanished path
                // (never at a re-resolved live copy — the == guards that).
                if self.local_files.get(&hashed.root) == Some(&path) {
                    self.local_files.remove(&hashed.root);
                }
            }
        }
        self.hash_cache = Arc::new(cache);
    }

    /// Persist one library-scan hash result to SQLite immediately, but
    /// only fold it into the in-memory `hash_cache` once
    /// [`SCAN_COMMIT_BATCH`] results have piled up (or
    /// [`Self::flush_scan_hash_commits`] is called explicitly, at scan
    /// completion) -- see [`SCAN_COMMIT_BATCH`] for why.
    fn queue_scan_hash_commit(&mut self, path: PathBuf, mtime: i64, hash: Ed2kFileHash) {
        let now = (self.clock)() as i64;
        if let Err(e) = self.storage.upsert_hash_cache(&path, mtime, &hash, now) {
            tracing::error!("persisting hash cache: {e}");
        }
        self.scan_pending_commits.push((path, mtime, hash));
        if self.scan_pending_commits.len() >= SCAN_COMMIT_BATCH {
            self.flush_scan_hash_commits();
        }
    }

    /// Fold any buffered scan hash results into `hash_cache` in one clone.
    fn flush_scan_hash_commits(&mut self) {
        if self.scan_pending_commits.is_empty() {
            return;
        }
        let mut cache = (*self.hash_cache).clone();
        for (path, mtime, hash) in self.scan_pending_commits.drain(..) {
            cache.insert(path, (mtime, hash));
        }
        self.hash_cache = Arc::new(cache);
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
                let path = path.clone();
                self.hash_manual_mapping(file, path.clone());
                let _ = self
                    .out
                    .send(FileOutput::Resolved {
                        file,
                        resolution: Resolution::Verified(path),
                    })
                    .await;
                return;
            }
            tracing::info!(path = %path.display(), "manual mapping points at nothing; re-matching");
        }
        let roots = self.media_roots.clone();
        let cache = Arc::clone(&self.hash_cache);
        // A completed download lives hash-named in the cache; offer it as
        // a by-hash candidate (the filename search can't find it).
        let cache_candidate = Some(self.download_path(file));
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let (resolution, fresh) =
                resolve_with_cache(&filename, file, &roots, &cache, cache_candidate);
            tracing::debug!(
                filename,
                elapsed_ms = started.elapsed().as_millis() as u64,
                fresh_hashes = fresh.len(),
                ?resolution,
                "file resolution finished"
            );
            let _ = done_tx.blocking_send(Done::Resolved {
                file,
                filename,
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
        self.local_files.insert(file, path.clone());
        self.hash_manual_mapping(file, path.clone());
        let _ = self
            .out
            .send(FileOutput::Resolved {
                file,
                resolution: Resolution::Verified(path),
            })
            .await;
    }

    /// Hash a manually-mapped file in the background so we can serve its
    /// block hashes to peers. A manual mapping is filename-trusted and
    /// often lives outside the media roots, so neither `resolve` nor the
    /// library scan ever hashes it — without this we advertise the file
    /// `Ready` but `serve_block_hashes` has nothing to send, wedging any
    /// downloader that picks us as a source (design.md File Matching 4a: a
    /// manual map is a servable local copy). The hash is committed (in
    /// `Done::ManualHashed`) only if it actually matches the mapped hash,
    /// so a different encode is never served under this file's identity.
    fn hash_manual_mapping(&self, file: Ed2kHash, path: PathBuf) {
        // Already cached with matching content? Nothing to do.
        if let Some((_, hashed)) = self.hash_cache.get(&path)
            && hashed.root == file
        {
            return;
        }
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let mtime = std::fs::metadata(&path)
                .ok()
                .as_ref()
                .and_then(mtime_millis);
            let result = std::fs::File::open(&path).and_then(ed2k_hash_reader);
            let _ = done_tx.blocking_send(Done::ManualHashed {
                file,
                path,
                mtime,
                result,
            });
        });
    }

    async fn archive(&mut self, file: Ed2kHash, series_name: Option<String>, filename: String) {
        let result = self.archive_inner(file, series_name, &filename);
        if let Ok(new_path) = &result {
            tracing::info!(path = %new_path.display(), "archived cached file into the library");
        }
        let _ = self.out.send(FileOutput::Archived { file, result }).await;
    }

    fn archive_inner(
        &mut self,
        file: Ed2kHash,
        series_name: Option<String>,
        filename: &str,
    ) -> Result<PathBuf, String> {
        // `[Series name]/[Original filename]` under the download root.
        // AniDB models each season as its own anime, so a single series
        // name is effectively one season's folder; the explicit
        // "Season #" the design mentions is deferred with that note.
        let download_root = self
            .media_roots
            .first()
            .ok_or("no download root configured")?;
        let folder = sanitize_component(series_name.as_deref().unwrap_or("Unsorted"));
        let dest = download_root
            .join(folder)
            .join(sanitize_component(filename));
        let entries = self.storage.cache_entries().map_err(|e| e.to_string())?;
        let entry = entries
            .iter()
            .find(|entry| entry.hash == file)
            .ok_or("not a cached download")?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
        }
        move_file(&entry.path, &dest).map_err(|e| e.to_string())?;
        self.storage
            .remove_cache_entry(file)
            .map_err(|e| e.to_string())?;
        // The hash is content-derived and unchanged; re-key the cache
        // row to the new path (mtime may differ after a cross-device
        // copy).
        if let Err(e) = self.storage.remove_hash_cache(&entry.path) {
            tracing::error!("hash-cache cleanup after archive: {e}");
        }
        // The archived file is still a held, servable copy -- only its
        // path changed. Re-point the servable map at the new location, in
        // lockstep with the hash_cache re-key below; leaving `local_files`
        // on the now-deleted cache path makes the serve path read a dead
        // file and flip us to Missing for a file we still hold.
        self.local_files.insert(file, dest.to_path_buf());
        let mut cache = (*self.hash_cache).clone();
        if let Some((_, hash)) = cache.remove(&entry.path) {
            let now = (self.clock)() as i64;
            if let Ok(metadata) = std::fs::metadata(&dest)
                && let Some(mtime) = mtime_millis(&metadata)
            {
                if let Err(e) = self.storage.upsert_hash_cache(&dest, mtime, &hash, now) {
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
            if !evictable(
                now,
                self.retention,
                &entry,
                watched,
                protected.contains(&entry.hash),
            ) {
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
            // Drop it from the in-memory servable set too: the file is gone
            // from disk, so we must not keep advertising/serving it. Without
            // this, local_files still points at the deleted path until a
            // serve request or re-resolve self-heals it (a redundant serve
            // attempt + a spurious Missing re-emit).
            self.local_files.remove(&entry.hash);
            evicted.push(entry.hash);
        }
        if !evicted.is_empty() {
            let _ = self.out.send(FileOutput::Evicted { files: evicted }).await;
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

/// Make a string safe as a single path component: replace separators
/// and characters that would escape the directory or upset Windows.
/// Empty input becomes `"Unsorted"`.
fn sanitize_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "Unsorted".to_string()
    } else {
        trimmed.to_string()
    }
}

/// How often the actor runs snub/refill maintenance and drains the
/// serve queue (a safety net; data arrival drives refill directly).
const DOWNLOAD_TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// A token-bucket upload pacer (bytes). Unlimited when no cap is set —
/// the seeder default. A burst of up to one second is allowed.
struct UploadLimiter {
    /// Bytes/sec cap; `None` = unlimited.
    limit: Option<u64>,
    /// Available bytes.
    tokens: f64,
    /// Last refill, shared-clock millis.
    last: u64,
}

impl UploadLimiter {
    fn new(limit: Option<u64>) -> Self {
        UploadLimiter {
            limit,
            tokens: limit.unwrap_or(0) as f64,
            last: 0,
        }
    }

    /// Try to spend `bytes` at `now`. `true` (and spends) if the budget
    /// allows; `false` to defer. Always `true` when unlimited.
    fn try_take(&mut self, bytes: u64, now: u64) -> bool {
        let Some(limit) = self.limit else {
            return true;
        };
        let limit = limit as f64;
        let elapsed = now.saturating_sub(self.last) as f64 / 1000.0;
        self.tokens = (self.tokens + elapsed * limit).min(limit);
        self.last = now;
        // A chunk larger than a full second's budget (an absurdly low
        // cap) is allowed when the bucket is full, so a download never
        // wedges; the debt is bounded by the refill clamp above.
        let need = (bytes as f64).min(limit);
        if self.tokens >= need {
            self.tokens -= bytes as f64;
            true
        } else {
            false
        }
    }
}

/// Read a byte range from a file (for serving a chunk).
fn read_range(path: &Path, range: std::ops::Range<u64>) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(range.start))?;
    let mut buf = vec![0u8; (range.end - range.start) as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
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
    cache_candidate: Option<PathBuf>,
) -> (Resolution, Vec<(PathBuf, i64, Ed2kFileHash)>) {
    let mut fresh = Vec::new();
    let mut mismatch = None;
    // A completed download is hash-named in the cache dir; check it by
    // hash first (the filename search below can never match it). A
    // content-addressed hit is the strongest possible verification.
    if let Some(candidate) = cache_candidate
        && let Some(root) = candidate_root(&candidate, cache, &mut fresh)
        && root == expected
    {
        return (Resolution::Verified(candidate), fresh);
    }
    for candidate in find_by_name(filename, roots) {
        let Some(root) = candidate_root(&candidate, cache, &mut fresh) else {
            continue;
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

/// The ed2k root of `candidate`: trusted from the hash cache when
/// (mtime, size) match, otherwise hashed for real exactly once (the
/// fresh hash is pushed to `fresh` for caching). `None` if unreadable.
fn candidate_root(
    candidate: &Path,
    cache: &HashMap<PathBuf, (i64, Ed2kFileHash)>,
    fresh: &mut Vec<(PathBuf, i64, Ed2kFileHash)>,
) -> Option<Ed2kHash> {
    let metadata = match std::fs::metadata(candidate) {
        Ok(metadata) => metadata,
        Err(e) => {
            tracing::debug!(path = %candidate.display(), "unreadable candidate: {e}");
            return None;
        }
    };
    let mtime = mtime_millis(&metadata);
    let cached_root = mtime.and_then(|mtime| {
        cache.get(candidate).and_then(|(cached_mtime, hash)| {
            (*cached_mtime == mtime && hash.size_bytes == metadata.len()).then_some(hash.root)
        })
    });
    match cached_root {
        Some(root) => Some(root),
        None => {
            // Cache miss or stale mtime: hash for real, once.
            match std::fs::File::open(candidate).and_then(ed2k_hash_reader) {
                Ok(hashed) => {
                    let root = hashed.root;
                    if let Some(mtime) = mtime {
                        fresh.push((candidate.to_path_buf(), mtime, hashed));
                    }
                    Some(root)
                }
                Err(e) => {
                    tracing::debug!(path = %candidate.display(), "unreadable candidate: {e}");
                    None
                }
            }
        }
    }
}

/// Video container extensions the library scan considers. A whole-library
/// scan must not hash `.nfo`/`.jpg`/subtitle files: they'd waste IO and
/// pollute the catalog and AniDB lookup set with junk.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "mov", "webm", "ts", "m4v", "wmv", "flv", "mpg", "mpeg", "ogm", "m2ts",
];

/// Whether `path` looks like a video file (case-insensitive extension).
fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| VIDEO_EXTENSIONS.contains(&ext.as_str()))
}

/// Directory names that organise a series' files but are not the series
/// itself — season/disc folders and generic library containers. Matched
/// case-insensitively, with an optional trailing number (`Season 1`, `CD2`).
const STRUCTURAL_DIR_WORDS: &[&str] = &[
    // Season / disc / part groupings.
    "season",
    "s",
    "saison",
    "disc",
    "disk",
    "cd",
    "dvd",
    "bd",
    "vol",
    "volume",
    "part",
    "pt",
    "special",
    "specials",
    "extra",
    "extras",
    "ova",
    "bdmv",
    "stream",
    "video_ts",
    // Generic library containers — using one of these as a series name would
    // collapse an entire mixed folder (e.g. a `Movies` dump) into one entry.
    "anime",
    "movies",
    "movie",
    "videos",
    "video",
    "downloads",
    "download",
    "media",
    "tv",
    "shows",
    "series",
    "watch",
    "incoming",
    "complete",
    "seeding",
];

/// True if `name` is a structural/container directory rather than a series
/// title: a season/disc folder, a purely-numeric folder, a generic library
/// container, or anything with no letters / shorter than two characters.
fn is_structural_dir(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.len() < 2 || !trimmed.chars().any(|c| c.is_alphabetic()) {
        return true;
    }
    // A season/disc word optionally followed by a number and separators,
    // e.g. "Season 1", "S01", "Disc-2", "CD 3".
    let lower = trimmed.to_ascii_lowercase();
    let word_end = lower
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(lower.len());
    let (word, rest) = lower.split_at(word_end);
    let rest_is_number = rest
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '.' | '_' | '-'));
    STRUCTURAL_DIR_WORDS.contains(&word) && rest_is_number
}

/// A title-like directory name for `path`, relative to the media `root` it
/// was found under: walk the ancestor directories between the root and the
/// file from deepest to shallowest and return the first that isn't a
/// structural/container folder (see [`is_structural_dir`]). `None` when the
/// file sits directly in the root or every ancestor is structural — the
/// server then falls back to the filename stem.
fn dir_series_hint(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    // Directory components between the root and the filename, shallow→deep.
    let dirs: Vec<&std::ffi::OsStr> = rel
        .parent()?
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    dirs.iter()
        .rev()
        .map(|s| s.to_string_lossy())
        .find(|name| !is_structural_dir(name))
        .map(|name| name.trim().to_string())
}

/// Breadth-first walk over every regular file under `roots`, **following
/// symlinks** — to both directories and files — so a library laid out with
/// symlinked series/season folders is fully visited. Cycle protection: each
/// directory is canonicalized and recorded in a shared `visited` set before
/// it is read, so a symlink pointing back into the tree (or two roots that
/// overlap) is walked at most once. The callback receives each file's path
/// (as reached, i.e. through the symlink, not its canonical target) and the
/// media root it was found under; root order is preserved. Unreadable or
/// uncanonicalizable directories are skipped with a debug log, and dangling
/// symlinks are skipped silently.
fn walk_files(roots: &[PathBuf], mut on_file: impl FnMut(PathBuf, &Path)) {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        let mut queue = std::collections::VecDeque::from([root.clone()]);
        while let Some(dir) = queue.pop_front() {
            // Cycle guard: canonicalize (resolving symlinks in the path) and
            // skip a real directory we've already walked.
            match std::fs::canonicalize(&dir) {
                Ok(canon) => {
                    if !visited.insert(canon) {
                        continue;
                    }
                }
                Err(e) => {
                    tracing::debug!(dir = %dir.display(), "uncanonicalizable directory: {e}");
                    continue;
                }
            }
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
                // `DirEntry::file_type()` does not follow symlinks; resolve the
                // link target's type so symlinked dirs/files are traversed.
                let resolved = if file_type.is_symlink() {
                    match std::fs::metadata(&path) {
                        Ok(meta) => meta.file_type(),
                        Err(_) => continue, // dangling symlink
                    }
                } else {
                    file_type
                };
                if resolved.is_dir() {
                    queue.push_back(path);
                } else if resolved.is_file() {
                    on_file(path, root);
                }
            }
        }
    }
}

/// Walk every media root, classifying each video file as a hash-cache hit
/// (a known root, returned immediately) or a worklist item (new or
/// changed since the last scan, needing a hash). Uses the shared
/// symlink-following [`walk_files`] traversal, visiting every video file.
fn scan_library(
    roots: &[PathBuf],
    cache: &HashMap<PathBuf, (i64, Ed2kFileHash)>,
) -> (
    Vec<IndexedFile>,
    std::collections::VecDeque<ScanItem>,
    Vec<PathBuf>,
) {
    let mut hits = Vec::new();
    let mut worklist = std::collections::VecDeque::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    walk_files(roots, |path, root| {
        if !is_video_file(&path) {
            return;
        }
        seen.insert(path.clone());
        // `std::fs::metadata` follows symlinks, so a symlinked video reports
        // its target's mtime/size (not the link's).
        let Ok(metadata) = std::fs::metadata(&path) else {
            return;
        };
        let Some(mtime) = mtime_millis(&metadata) else {
            return;
        };
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let series_hint = dir_series_hint(&path, root);
        match cache.get(&path) {
            // Unchanged since we hashed it: trust the cache.
            Some((cached_mtime, hash))
                if *cached_mtime == mtime && hash.size_bytes == metadata.len() =>
            {
                hits.push(IndexedFile {
                    hash: hash.root,
                    size: hash.size_bytes,
                    filename,
                    mtime: *cached_mtime,
                    series_hint,
                });
            }
            // New or changed: needs a (re)hash.
            _ => worklist.push_back(ScanItem {
                path,
                mtime,
                filename,
                series_hint,
            }),
        }
    });
    // Index rows for files that vanished from under the roots (moved or
    // deleted behind the app's back) — the disk is the truth, the index
    // follows it. The `seen` check spares a stat per live row; the
    // `exists` double-check protects rows the walk deliberately skips
    // (e.g. a non-video file hashed via playlist-add) and files created
    // mid-walk. Rows outside the roots (the download cache, removed
    // roots) are none of the scan's business.
    let stale: Vec<PathBuf> = cache
        .keys()
        .filter(|path| roots.iter().any(|root| path.starts_with(root)))
        .filter(|path| !seen.contains(*path) && !path.exists())
        .cloned()
        .collect();
    (hits, worklist, stale)
}

/// Every file named exactly `filename` under the roots, in breadth-first
/// root order. Uses the shared symlink-following [`walk_files`] traversal,
/// so it stays consistent with [`scan_library`]'s indexing.
fn find_by_name(filename: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk_files(roots, |path, _root| {
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy() == filename)
        {
            found.push(path);
        }
    });
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
            None,
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
            None,
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
            None,
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
            None,
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

        let (resolution, fresh) = resolve_with_cache(
            "ep1.mkv",
            expected,
            &[root.path().to_path_buf()],
            &cache,
            None,
        );
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

        let (resolution, fresh) = resolve_with_cache(
            "ep1.mkv",
            expected,
            &[root.path().to_path_buf()],
            &cache,
            None,
        );
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
    fn sanitize_component_contains_path_separators() {
        // A normal name is untouched.
        assert_eq!(sanitize_component("Frieren"), "Frieren");
        assert_eq!(sanitize_component("Fate/stay night"), "Fate_stay night");
        assert_eq!(sanitize_component(""), "Unsorted");
        assert_eq!(sanitize_component("   "), "Unsorted");
        // The security property: no separator survives and the result
        // can never be a traversal component.
        for evil in ["../../etc/passwd", "..\\..\\windows", "/abs/path", "."] {
            let s = sanitize_component(evil);
            assert!(!s.contains('/') && !s.contains('\\'), "{s:?}");
            assert!(s != "." && s != "..", "{s:?}");
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
        assert!(evictable(
            1_000,
            CacheRetention::AfterWatch,
            &entry,
            true,
            false
        ));

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
        _cache_dir: Option<tempfile::TempDir>,
    }

    /// Spawn the actor against a caller-supplied cache dir (so a test can
    /// pre-populate it before startup reconciliation runs).
    fn spawn_rig_at(
        storage: Storage,
        roots: Vec<PathBuf>,
        retention: CacheRetention,
        cache_dir: PathBuf,
    ) -> Rig {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (out_tx, out_rx) = mpsc::channel(64);
        tokio::spawn(run(
            FileConfig {
                storage,
                media_roots: roots,
                retention,
                cache_dir,
                clock: test_clock(),
                download: DownloadConfig::default(),
                upload_limit: None,
                // No timer-driven scan in tests; drive via RescanLibrary.
                scan_interval: None,
                // Short deferral window so recheck/deferral tests run fast.
                scan_transfer_quiet: Duration::from_secs(2),
            },
            cmd_rx,
            out_tx,
        ));
        Rig {
            commands: cmd_tx,
            outputs: out_rx,
            _cache_dir: None,
        }
    }

    fn spawn_rig(storage: Storage, roots: Vec<PathBuf>, retention: CacheRetention) -> Rig {
        let cache_dir = tempfile::tempdir().unwrap();
        let mut rig = spawn_rig_at(storage, roots, retention, cache_dir.path().to_path_buf());
        rig._cache_dir = Some(cache_dir);
        rig
    }

    async fn next_output(rig: &mut Rig) -> FileOutput {
        tokio::time::timeout(Duration::from_secs(10), rig.outputs.recv())
            .await
            .expect("output timeout")
            .expect("actor gone")
    }

    /// Regression (2026-07-03): a full library scan reported one
    /// `Done::LibraryHashed` per file, and each one used to clone the
    /// *entire* `hash_cache` map before replacing it — O(n) work per
    /// file, O(n^2) for a scan of n files. On the primary seeder's
    /// terabyte-scale library this produced a burst of ever-larger
    /// transient allocations that fragmented the allocator badly enough
    /// that `malloc_trim` recovered ~360MB of RSS. Scan commits must be
    /// batched into a bounded number of map rebuilds, not one per file.
    #[test]
    fn library_scan_batches_hash_cache_commits_instead_of_cloning_per_file() {
        let db = tempfile::tempdir().unwrap();
        let storage = Storage::open(&db.path().join("test.db")).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let (out_tx, _out_rx) = mpsc::channel(1024);
        let (done_tx, _done_rx) = mpsc::channel(1024);
        let mut actor = Actor::new(
            FileConfig {
                storage,
                media_roots: vec![],
                retention: CacheRetention::default(),
                cache_dir: cache_dir.path().to_path_buf(),
                clock: test_clock(),
                download: DownloadConfig::default(),
                upload_limit: None,
                scan_interval: None,
                scan_transfer_quiet: Duration::from_secs(2),
            },
            out_tx,
            done_tx,
        )
        .unwrap();

        let n = 250;
        let mut rebuilds = 0usize;
        let mut last_ptr = Arc::as_ptr(&actor.hash_cache);
        for i in 0..n {
            let path = PathBuf::from(format!("/media/ep{i}.mkv"));
            let hash = ed2k_hash_bytes(format!("episode {i}").as_bytes());
            actor.queue_scan_hash_commit(path, 1, hash);
            let ptr = Arc::as_ptr(&actor.hash_cache);
            if ptr != last_ptr {
                rebuilds += 1;
                last_ptr = ptr;
            }
        }
        actor.flush_scan_hash_commits();
        if Arc::as_ptr(&actor.hash_cache) != last_ptr {
            rebuilds += 1;
        }

        assert_eq!(
            actor.hash_cache.len(),
            n,
            "every scanned file must end up in the cache"
        );
        assert!(
            rebuilds < n,
            "hash_cache was rebuilt {rebuilds} times for {n} files -- \
             it must not be cloned once per scanned file"
        );
    }

    /// Spec (Download Cache and Retention): an evictable file is deleted
    /// `cache_retention` after its *last access*. last_access is written at
    /// download time; resolving (loading) a cached file again must bump it,
    /// or the retention window wrongly runs from download time and a
    /// re-watched file is evicted on schedule regardless.
    #[tokio::test]
    async fn resolving_a_cached_file_bumps_last_access() {
        let cache = tempfile::tempdir().unwrap();
        let contents = b"a cached episode".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        // The download cache is hash-named.
        let cached_path = cache.path().join(hashed.root.to_string());
        std::fs::write(&cached_path, contents).unwrap();

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        // Downloaded "long ago": last_access == 1.
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

        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().to_path_buf(),
        );
        rig.commands
            .send(FileCommand::Resolve {
                file: hashed.root,
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

        // last_access bumped from download time (1) to the resolve clock.
        let now: i64 = 1_700_000_000_000;
        let check = Storage::open(&db_path).unwrap();
        let rows = check.cache_entries().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].last_access, now,
            "resolving a cached file must bump last_access"
        );

        // ...so eviction is deferred: a watched file under the default
        // Keep(1 week) retention, "downloaded" long ago, is NOT evictable,
        // because its last access is now.
        let week = Duration::from_secs(7 * 24 * 3600);
        assert!(!evictable(
            now,
            CacheRetention::Keep(week),
            &rows[0],
            true,
            false,
        ));
        drop(cache);
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

    /// Regression (2026-07-02, "Release that Witch ep 5"): a file moved
    /// between directories under a media root leaves its old hash_cache
    /// row behind forever — the scan only ever adds rows. The stale row
    /// then pollutes everything built on the library index (browser
    /// search ghosts, anchor resolution landing in the wrong directory).
    /// A rescan must prune index rows whose files vanished from under
    /// the roots, in SQLite and in the in-memory map.
    #[tokio::test]
    async fn rescan_prunes_vanished_files_from_the_index() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode five".as_slice();
        let moved = write(root.path(), "Fangkai/ep5.mkv", contents);
        let hashed = ed2k_hash_bytes(contents);
        let mtime = mtime_millis(&std::fs::metadata(&moved).unwrap()).unwrap();

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        // The current location is indexed…
        storage
            .upsert_hash_cache(&moved, mtime, &hashed, 1)
            .unwrap();
        // …and so is the old, pre-move location (no file there anymore).
        let stale = root.path().join("ep5.mkv");
        storage
            .upsert_hash_cache(&stale, mtime, &hashed, 1)
            .unwrap();
        // A row outside the media roots (a download-cache file) must be
        // left alone — the scan has no opinion on the cache.
        let outside = PathBuf::from("/nonexistent-cache/aabbcc");
        storage
            .upsert_hash_cache(&outside, mtime, &hashed, 1)
            .unwrap();

        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();
        // The walk reports the surviving file as indexed.
        match next_output(&mut rig).await {
            FileOutput::LibraryIndexed { files } => {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].filename, "ep5.mkv");
            }
            other => panic!("unexpected output: {other:?}"),
        }
        // The stale row is pruned; the live and out-of-root rows survive.
        let check = Storage::open(&db_path).unwrap();
        let paths: Vec<PathBuf> = check
            .hash_cache()
            .unwrap()
            .into_iter()
            .map(|row| row.path)
            .collect();
        assert!(!paths.contains(&stale), "stale row must be pruned");
        assert!(paths.contains(&moved));
        assert!(paths.contains(&outside));
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

    /// Regression: a manually-mapped file must be servable to peers. A manual
    /// mapping is filename-trusted and often lives outside the media roots, so
    /// neither resolve nor the library scan ever hashes it. Before the fix the
    /// holder advertised the file Ready but never cached its block hashes, so
    /// serve_block_hashes silently bailed and any downloader that picked this
    /// holder wedged. Now the mapping is hashed in the background and served.
    #[tokio::test]
    async fn manual_mapping_becomes_servable_to_peers() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"the real episode bytes".as_slice();
        // Named differently from the playlist entry and outside any root.
        let path = write(root.path(), "renamed.mkv", contents);
        let hashed = ed2k_hash_bytes(contents);

        let mut rig = spawn_rig(
            Storage::open_in_memory().unwrap(),
            vec![], // path is in no media root
            CacheRetention::default(),
        );
        rig.commands
            .send(FileCommand::SetManualMapping {
                file: hashed.root,
                path: path.clone(),
                series: None,
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { file, resolution } => {
                assert_eq!(file, hashed.root);
                assert_eq!(resolution, Resolution::Verified(path));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        // The hash runs on the blocking pool (no virtual-time hook), so poll
        // the serve path until the mapping is cached — a tiny file, so this
        // resolves near-instantly; the budget only guards against a genuine
        // never-servable regression.
        let mut served = false;
        for _ in 0..100 {
            rig.commands
                .send(FileCommand::PeerMessage {
                    from: PeerId::new("peer7"),
                    message: Box::new(PeerMessage::BlockHashRequest { file: hashed.root }),
                })
                .await
                .unwrap();
            match tokio::time::timeout(Duration::from_millis(50), next_output(&mut rig)).await {
                Ok(FileOutput::SendPeer { message, .. }) => match *message {
                    PeerMessage::BlockHashes { file, hashes } => {
                        assert_eq!(file, hashed.root);
                        assert_eq!(hashes.len(), hashed.blocks.len());
                        served = true;
                        break;
                    }
                    // A stray follow-up bitfield from an earlier serve: ignore.
                    PeerMessage::FileAvailability { .. } => {}
                    other => panic!("unexpected peer message: {other:?}"),
                },
                Ok(other) => panic!("unexpected output: {other:?}"),
                Err(_) => {} // not hashed yet; retry
            }
        }
        assert!(served, "manual mapping never became servable to peers");
    }

    /// Regression: block hashes must never be served under a file's identity
    /// when the local copy hashes to something else (a manual map to a
    /// different encode). Before the guard, serve_block_hashes handed the
    /// mapped file's block hashes to a downloader requesting a different hash,
    /// propagating a mismatched encode to the group.
    #[tokio::test]
    async fn block_hashes_not_served_for_mismatched_content() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"some other encode".as_slice();
        let path = write(root.path(), "other.mkv", contents);
        let real = ed2k_hash_bytes(contents);

        // The path is already known to hash to `real` (e.g. a prior scan).
        let storage = Storage::open_in_memory().unwrap();
        let mtime = mtime_millis(&std::fs::metadata(&path).unwrap()).unwrap();
        storage.upsert_hash_cache(&path, mtime, &real, 1).unwrap();

        let mut rig = spawn_rig(storage, vec![], CacheRetention::default());
        // Map a *different* playlist hash to this file (content mismatch).
        let requested = hash(1);
        assert_ne!(requested, real.root);
        rig.commands
            .send(FileCommand::SetManualMapping {
                file: requested,
                path: path.clone(),
                series: None,
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { resolution, .. } => {
                assert_eq!(resolution, Resolution::Verified(path));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        // A peer solicits the requested (mismatched) hash. We must not serve.
        rig.commands
            .send(FileCommand::PeerMessage {
                from: PeerId::new("peer7"),
                message: Box::new(PeerMessage::BlockHashRequest { file: requested }),
            })
            .await
            .unwrap();
        // No output at all: the guard drops it. (A served response would be a
        // SendPeer here.)
        match tokio::time::timeout(Duration::from_millis(200), next_output(&mut rig)).await {
            Err(_) => {} // expected: nothing served
            Ok(FileOutput::SendPeer { message, .. }) => {
                panic!("served block hashes for mismatched content: {message:?}")
            }
            Ok(other) => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn record_watched_writes_history_and_emits_refresh() {
        // Regression: recording a watch must emit FileOutput::WatchRecorded
        // so the bridge pushes a fresh snapshot — otherwise Recent Series
        // never reflects the just-watched episode (2026-06-14).
        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        let mut rig = spawn_rig(storage, vec![], CacheRetention::default());

        rig.commands
            .send(FileCommand::RecordWatched(WatchRecord {
                hash: hash(9),
                series_id: Some(AniDbSeriesId(5)),
                series_name: Some("Frieren".into()),
                filename: "frieren-01.mkv".into(),
                watched_at: 42,
            }))
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::WatchRecorded => {}
            other => panic!("unexpected output: {other:?}"),
        }

        // The record actually landed in watch history.
        let verify = Storage::open(&db_path).unwrap();
        let record = verify.watched(hash(9)).unwrap().expect("watch recorded");
        assert_eq!(record.series_id, Some(AniDbSeriesId(5)));
        assert_eq!(record.watched_at, 42);
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

        // Download root is the first media root; the actor builds
        // <root>/<series>/<filename>.
        let dest = library.path().join("Frieren/ep1.mkv");
        let mut rig = spawn_rig(
            storage,
            vec![library.path().to_path_buf()],
            CacheRetention::default(),
        );
        rig.commands
            .send(FileCommand::Archive {
                file: hashed.root,
                series_name: Some("Frieren".into()),
                filename: "ep1.mkv".into(),
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
                series_name: None,
                filename: "nope.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Archived { result, .. } => assert!(result.is_err()),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    /// Regression: archiving a cached download must keep it servable. The
    /// file is still held (now at its library path), so a peer that picked
    /// us as a source must still get its block hashes -- and we must NOT
    /// spuriously flip our own availability to Missing. The bug: archive
    /// re-keyed `hash_cache` to the new path but forgot `local_files`, so
    /// the serve path read the now-deleted cache path, saw it gone, and
    /// called `lost_local_file` -> Missing (gating the whole group if the
    /// archived file is now-playing).
    #[tokio::test]
    async fn archived_file_stays_servable_to_peers() {
        let cache = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let contents = b"a cached episode".as_slice();
        let cached_path = write(cache.path(), "ep1.mkv", contents);
        let hashed = ed2k_hash_bytes(contents);

        let storage = Storage::open_in_memory().unwrap();
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

        let dest = library.path().join("Frieren/ep1.mkv");
        let mut rig = spawn_rig(
            storage,
            vec![library.path().to_path_buf()],
            CacheRetention::default(),
        );

        // Archive moves <cache>/ep1.mkv -> <library>/Frieren/ep1.mkv.
        // Startup reconciliation has already registered the cache file as a
        // servable copy before the command loop runs, so the move must
        // re-point that registration at `dest`.
        rig.commands
            .send(FileCommand::Archive {
                file: hashed.root,
                series_name: Some("Frieren".into()),
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Archived { result, .. } => assert_eq!(result.unwrap(), dest),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(dest.is_file());
        assert!(!cached_path.exists());

        // A peer that advertised-Ready picked us as a source asks for the
        // block hashes. We still hold the file (at `dest`), so we serve it.
        rig.commands
            .send(FileCommand::PeerMessage {
                from: PeerId::new("peer7"),
                message: Box::new(PeerMessage::BlockHashRequest { file: hashed.root }),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::SendPeer { message, .. } => match *message {
                PeerMessage::BlockHashes { file, .. } => assert_eq!(file, hashed.root),
                other => panic!("expected block hashes, got {other:?}"),
            },
            FileOutput::Availability { availability, .. } => panic!(
                "archived file spuriously flipped to {availability:?} on a peer serve request"
            ),
            other => panic!("unexpected output: {other:?}"),
        }
        // The follow-up complete-bitfield advertisement confirms we still
        // present as a full source for the archived file.
        match next_output(&mut rig).await {
            FileOutput::SendPeer { message, .. } => {
                assert!(matches!(*message, PeerMessage::FileAvailability { .. }));
            }
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
                    // Real size: reconciliation prunes rows whose size
                    // disagrees with the file on disk.
                    size_bytes: std::fs::metadata(path).unwrap().len(),
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

    /// Regression: an eviction pass must drop the evicted file from the
    /// in-memory servable set (local_files), not just the DB and hash cache.
    /// Before the fix local_files still pointed at the deleted path, so a
    /// peer's block-hash request for the evicted file found it "held", hit
    /// the now-missing path, and re-emitted a spurious Missing.
    #[tokio::test]
    async fn eviction_drops_the_file_from_the_servable_set() {
        let cache = tempfile::tempdir().unwrap();
        let contents = b"a watched episode".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        let cached_path = cache.path().join(hashed.root.to_string());
        std::fs::write(&cached_path, contents).unwrap();

        let storage = Storage::open_in_memory().unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hashed.root,
                path: cached_path.clone(),
                size_bytes: contents.len() as u64,
                last_access: 0,
            })
            .unwrap();
        let mtime = mtime_millis(&std::fs::metadata(&cached_path).unwrap()).unwrap();
        storage
            .upsert_hash_cache(&cached_path, mtime, &hashed, 0)
            .unwrap();
        // Personally watched, so it is evictable under AfterWatch retention.
        storage
            .record_watched(&WatchRecord {
                hash: hashed.root,
                series_id: None,
                series_name: None,
                filename: "ep.mkv".into(),
                watched_at: 1,
            })
            .unwrap();

        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::AfterWatch,
            cache.path().into(),
        );
        rig.commands
            .send(FileCommand::RunEviction {
                protected: HashSet::new(),
                group_watched: HashSet::new(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Evicted { files } => assert_eq!(files, vec![hashed.root]),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(!cached_path.exists());

        // A peer solicits the evicted file, then we issue an unrelated
        // Resolve as a sentinel. Post-fix the serve request is a silent bail
        // (we no longer hold it), so the sentinel's Resolved is the next
        // output; pre-fix the stale servable entry re-emitted a Missing for
        // the evicted hash first.
        rig.commands
            .send(FileCommand::PeerMessage {
                from: PeerId::new("peer7"),
                message: Box::new(PeerMessage::BlockHashRequest { file: hashed.root }),
            })
            .await
            .unwrap();
        rig.commands
            .send(FileCommand::Resolve {
                file: hash(9),
                filename: "sentinel.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { file, .. } => assert_eq!(
                file,
                hash(9),
                "serve request for the evicted file produced a spurious output"
            ),
            other => panic!("evicted file still in the servable set: {other:?}"),
        }
    }

    // ---- DB-vs-filesystem reconciliation (Phase 9A hardening).

    /// A completed download is stored hash-named in the cache dir, with
    /// its filename in *no* media root. After a restart it must still
    /// resolve — by hash, not by the (non-matching) filename search.
    #[tokio::test]
    async fn cached_download_resolves_by_hash_after_restart() {
        let cache = tempfile::tempdir().unwrap();
        let contents = b"a cached episode".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        // The download cache names files after the hash.
        let cached_path = cache.path().join(hashed.root.to_string());
        std::fs::write(&cached_path, contents).unwrap();

        let storage = Storage::open_in_memory().unwrap();
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

        // No media roots at all: the filename can only ever be found in
        // the cache, and only by hash.
        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().into(),
        );
        rig.commands
            .send(FileCommand::Resolve {
                file: hashed.root,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { file, resolution } => {
                assert_eq!(file, hashed.root);
                assert_eq!(resolution, Resolution::Verified(cached_path));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    /// Startup reconciliation prunes cache rows whose file the user
    /// deleted (or truncated) out from under us, so the entry re-resolves
    /// to Missing and re-downloads; live rows are kept.
    #[tokio::test]
    async fn reconcile_prunes_dead_cache_rows_and_keeps_live() {
        let cache = tempfile::tempdir().unwrap();
        let contents = b"still here".as_slice();
        let live = cache.path().join(hash(1).to_string());
        std::fs::write(&live, contents).unwrap();
        let gone = cache.path().join(hash(2).to_string()); // never created

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hash(1),
                path: live.clone(),
                size_bytes: contents.len() as u64,
                last_access: 1,
            })
            .unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hash(2),
                path: gone.clone(),
                size_bytes: 10,
                last_access: 1,
            })
            .unwrap();

        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().into(),
        );
        // A resolve round-trip guarantees startup (and thus reconcile)
        // completed before we inspect the DB.
        rig.commands
            .send(FileCommand::Resolve {
                file: hash(99),
                filename: "absent.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { resolution, .. } => {
                assert_eq!(resolution, Resolution::NotFound);
            }
            other => panic!("unexpected output: {other:?}"),
        }

        let check = Storage::open(&db_path).unwrap();
        let rows = check.cache_entries().unwrap();
        assert_eq!(rows.len(), 1, "the dead row must be pruned");
        assert_eq!(rows[0].hash, hash(1));
    }

    /// Property: after reconciliation, every surviving `cache_entries`
    /// row points at an existing file of the recorded size. Deterministic
    /// over present / absent / wrong-size states.
    #[tokio::test]
    async fn reconcile_invariant_over_mixed_states() {
        let cache = tempfile::tempdir().unwrap();
        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();

        let mut expected_survivors = Vec::new();
        for i in 0u8..9 {
            let path = cache.path().join(hash(i).to_string());
            let body = vec![i; 32];
            let size = match i % 3 {
                0 => {
                    // present, correct size
                    std::fs::write(&path, &body).unwrap();
                    expected_survivors.push(hash(i));
                    body.len() as u64
                }
                1 => 32, // absent: file never written
                _ => {
                    // present, but the row lies about the size
                    std::fs::write(&path, &body).unwrap();
                    body.len() as u64 + 1
                }
            };
            storage
                .upsert_cache_entry(&CacheEntry {
                    hash: hash(i),
                    path,
                    size_bytes: size,
                    last_access: 1,
                })
                .unwrap();
        }

        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().into(),
        );
        rig.commands
            .send(FileCommand::Resolve {
                file: hash(200),
                filename: "absent.mkv".into(),
            })
            .await
            .unwrap();
        let _ = next_output(&mut rig).await;

        let check = Storage::open(&db_path).unwrap();
        let mut survivors: Vec<_> = check.cache_entries().unwrap();
        survivors.sort_by_key(|e| e.hash.0);
        expected_survivors.sort_by_key(|h| h.0);
        assert_eq!(
            survivors.iter().map(|e| e.hash).collect::<Vec<_>>(),
            expected_survivors
        );
        for entry in survivors {
            let metadata = std::fs::metadata(&entry.path).expect("survivor must exist");
            assert_eq!(metadata.len(), entry.size_bytes, "survivor size must match");
        }
    }

    /// Guard: if a file we still claim to hold has been deleted, a peer's
    /// block-hash request must not falsely advertise it; instead we drop
    /// it locally and flip our own availability to Missing.
    #[tokio::test]
    async fn serving_a_deleted_file_flips_to_missing() {
        let cache = tempfile::tempdir().unwrap();
        let contents = b"servable then gone".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        let cached_path = cache.path().join(hashed.root.to_string());
        std::fs::write(&cached_path, contents).unwrap();

        let storage = Storage::open_in_memory().unwrap();
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

        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().into(),
        );
        // Round-trip a resolve so startup reconciliation (which registers
        // the cached file as servable) has definitely run before we
        // delete it — otherwise the race makes the test flaky.
        rig.commands
            .send(FileCommand::Resolve {
                file: hash(123),
                filename: "absent.mkv".into(),
            })
            .await
            .unwrap();
        assert!(matches!(
            next_output(&mut rig).await,
            FileOutput::Resolved {
                resolution: Resolution::NotFound,
                ..
            }
        ));
        // The user nukes the cached file behind our back.
        std::fs::remove_file(&cached_path).unwrap();

        rig.commands
            .send(FileCommand::PeerMessage {
                from: PeerId::new("peer7"),
                message: Box::new(PeerMessage::BlockHashRequest { file: hashed.root }),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Availability { file, availability } => {
                assert_eq!(file, hashed.root);
                assert_eq!(availability, FileAvailability::Missing);
            }
            other => panic!("expected Missing, not a peer advertisement: {other:?}"),
        }
    }

    /// The load-failure runtime guard: `ForgetLocalFile` must do the same
    /// "drop + prune + flip to Missing" the serve-time guard does, so the
    /// file actor's bookkeeping (cache_entries, hash_cache) is pruned and
    /// the file re-resolves — not just flipped to Missing in synced state.
    #[tokio::test]
    async fn forget_local_file_drops_and_prunes_bookkeeping() {
        let cache = tempfile::tempdir().unwrap();
        let contents = b"held then gone".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        let cached_path = cache.path().join(hashed.root.to_string());
        std::fs::write(&cached_path, contents).unwrap();

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

        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().into(),
        );
        // Round-trip a resolve so startup reconciliation has registered the
        // cached file as a servable local copy before we forget it.
        rig.commands
            .send(FileCommand::Resolve {
                file: hashed.root,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        assert!(matches!(
            next_output(&mut rig).await,
            FileOutput::Resolved {
                resolution: Resolution::Verified(_),
                ..
            }
        ));

        // The player failed to load it (gone under us): forget the copy.
        rig.commands
            .send(FileCommand::ForgetLocalFile { file: hashed.root })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Availability { file, availability } => {
                assert_eq!(file, hashed.root);
                assert_eq!(availability, FileAvailability::Missing);
            }
            other => panic!("expected Missing after forget: {other:?}"),
        }

        // Bookkeeping pruned, mirroring the serve-time guard.
        let check = Storage::open(&db_path).unwrap();
        assert!(
            check.cache_entries().unwrap().is_empty(),
            "forget must prune the cache entry"
        );
        assert!(
            check.hash_cache().unwrap().is_empty(),
            "forget must prune the hash_cache row"
        );
        drop(cache);
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
                size_bytes: std::fs::metadata(&behind_path).unwrap().len(),
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

    // ---- media-library scan.

    /// Build an in-memory hash-cache map keyed by `path`, trusting the
    /// file's current mtime (so the entry counts as a cache hit).
    fn cache_of(path: &Path, contents: &[u8]) -> HashMap<PathBuf, (i64, Ed2kFileHash)> {
        let mtime = mtime_millis(&std::fs::metadata(path).unwrap()).unwrap();
        HashMap::from([(path.to_path_buf(), (mtime, ed2k_hash_bytes(contents)))])
    }

    #[test]
    fn scan_library_classifies_hits_worklist_and_skips_non_video() {
        let root = tempfile::tempdir().unwrap();
        let video = write(root.path(), "Frieren/ep1.mkv", b"episode one");
        // Junk files must never be hashed or indexed.
        write(root.path(), "Frieren/poster.jpg", b"jpeg");
        write(root.path(), "Frieren/ep1.nfo", b"<nfo/>");

        // Empty cache: the video needs hashing, the junk is ignored.
        let (hits, worklist, _stale) = scan_library(&[root.path().to_path_buf()], &HashMap::new());
        assert!(hits.is_empty());
        assert_eq!(worklist.len(), 1);
        assert_eq!(worklist[0].path, video);
        assert_eq!(worklist[0].filename, "ep1.mkv");

        // With the video already cached (matching mtime/size) it's a hit,
        // not re-hashed.
        let cache = cache_of(&video, b"episode one");
        let (hits, worklist, _stale) = scan_library(&[root.path().to_path_buf()], &cache);
        assert!(worklist.is_empty());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].hash, ed2k_hash_bytes(b"episode one").root);
        assert_eq!(hits[0].filename, "ep1.mkv");
    }

    #[test]
    fn scan_library_rehashes_a_changed_file() {
        let root = tempfile::tempdir().unwrap();
        let video = write(root.path(), "ep1.mkv", b"episode one");
        // Cache row records a *different* mtime than the file now has.
        let stale_mtime = mtime_millis(&std::fs::metadata(&video).unwrap()).unwrap() - 5_000;
        let cache = HashMap::from([(
            video.clone(),
            (stale_mtime, ed2k_hash_bytes(b"episode one")),
        )]);
        let (hits, worklist, _stale) = scan_library(&[root.path().to_path_buf()], &cache);
        assert!(hits.is_empty(), "stale mtime must not count as a hit");
        assert_eq!(worklist.len(), 1);
        assert_eq!(worklist[0].path, video);
    }

    // ---- Symlink traversal (Unix only; Windows symlink creation needs
    // privileges, and the deployment targets are NixOS/macOS).

    #[cfg(unix)]
    #[test]
    fn scan_library_follows_a_symlinked_directory() {
        // The real episode lives outside the media root; the root only
        // contains a symlink to its series directory.
        let base = tempfile::tempdir().unwrap();
        write(base.path(), "store/Frieren/ep1.mkv", b"episode one");
        let root = base.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(base.path().join("store/Frieren"), root.join("Frieren"))
            .unwrap();

        let (hits, worklist, _stale) = scan_library(std::slice::from_ref(&root), &HashMap::new());
        assert!(hits.is_empty());
        assert_eq!(
            worklist.len(),
            1,
            "video behind a symlinked dir must be seen"
        );
        assert_eq!(worklist[0].filename, "ep1.mkv");
        assert_eq!(worklist[0].path, root.join("Frieren/ep1.mkv"));
    }

    #[cfg(unix)]
    #[test]
    fn scan_library_follows_a_symlinked_file_with_target_metadata() {
        // A symlink that names a video, pointing at a real file elsewhere.
        let base = tempfile::tempdir().unwrap();
        let target = write(base.path(), "store/real.mkv", b"episode one");
        let root = base.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let link = root.join("ep1.mkv");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let (hits, worklist, _stale) = scan_library(std::slice::from_ref(&root), &HashMap::new());
        assert!(hits.is_empty());
        assert_eq!(worklist.len(), 1, "symlinked video file must be seen");
        assert_eq!(worklist[0].path, link);
        // mtime must come from the target, not the symlink itself.
        let target_mtime = mtime_millis(&std::fs::metadata(&target).unwrap()).unwrap();
        assert_eq!(worklist[0].mtime, target_mtime);
    }

    #[cfg(unix)]
    #[test]
    fn scan_library_terminates_on_a_symlink_cycle() {
        // A symlink pointing back at an ancestor must not loop forever, and
        // each real file is indexed exactly once.
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "Frieren/ep1.mkv", b"episode one");
        std::os::unix::fs::symlink(root.path(), root.path().join("Frieren/loop")).unwrap();

        let (_hits, worklist, _stale) = scan_library(&[root.path().to_path_buf()], &HashMap::new());
        assert_eq!(worklist.len(), 1, "cycle must not duplicate or hang");
        assert_eq!(worklist[0].filename, "ep1.mkv");
    }

    #[cfg(unix)]
    #[test]
    fn find_by_name_follows_a_symlinked_directory() {
        // Resolution must stay consistent with indexing: a file reachable
        // only through a symlinked dir is found by name.
        let base = tempfile::tempdir().unwrap();
        write(base.path(), "store/Frieren/ep1.mkv", b"episode one");
        let root = base.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(base.path().join("store/Frieren"), root.join("Frieren"))
            .unwrap();

        let found = find_by_name("ep1.mkv", std::slice::from_ref(&root));
        assert_eq!(found, vec![root.join("Frieren/ep1.mkv")]);
    }

    /// Drain outputs until a `LibraryIndexed` carrying `wanted` arrives.
    async fn await_indexed(rig: &mut Rig, wanted: Ed2kHash) {
        for _ in 0..50 {
            if let FileOutput::LibraryIndexed { files } = next_output(rig).await
                && files.iter().any(|f| f.hash == wanted)
            {
                return;
            }
        }
        panic!("never saw {wanted} indexed");
    }

    #[tokio::test]
    async fn rescan_indexes_video_files_end_to_end() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        write(root.path(), "Frieren/ep1.mkv", contents);
        write(root.path(), "Frieren/ep1.nfo", b"junk");
        let expected = ed2k_hash_bytes(contents).root;

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );

        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();
        await_indexed(&mut rig, expected).await;

        // The scan persisted the hash; a fresh connection sees it (so the
        // next scan is a cache hit, not a re-hash).
        let check = Storage::open(&db_path).unwrap();
        let cached = check.hash_cache().unwrap();
        assert!(cached.iter().any(|row| row.hash.root == expected));
        // The .nfo was never hashed.
        assert!(
            cached
                .iter()
                .all(|row| row.path.extension().unwrap() == "mkv")
        );
    }

    fn hint(rel: &str) -> Option<String> {
        let root = Path::new("/media/anime");
        dir_series_hint(&root.join(rel), root)
    }

    #[test]
    fn dir_series_hint_uses_the_containing_series_folder() {
        assert_eq!(
            hint("RahXephon/Season 1/Episode 43.mkv").as_deref(),
            Some("RahXephon")
        );
        assert_eq!(hint("RahXephon/ep01.mkv").as_deref(), Some("RahXephon"));
    }

    #[test]
    fn dir_series_hint_skips_structural_folders_to_the_title() {
        // Season *and* disc folders are skipped; the title is found above them.
        assert_eq!(
            hint("RahXephon/Season 1/Disc 1/ep.mkv").as_deref(),
            Some("RahXephon")
        );
        assert_eq!(hint("Show/S01/01.mkv").as_deref(), Some("Show"));
        assert_eq!(hint("Show/Specials/sp1.mkv").as_deref(), Some("Show"));
    }

    #[test]
    fn dir_series_hint_is_none_without_a_title_folder() {
        // File directly under a media root: no containing folder.
        assert_eq!(hint("loose.mkv"), None);
        // A generic container would over-group an unrelated dump.
        assert_eq!(hint("Movies/SomeFilm.mkv"), None);
        assert_eq!(hint("Anime/whatever.mkv"), None);
        // Only structural folders all the way down.
        assert_eq!(hint("Season 1/01.mkv"), None);
        assert_eq!(hint("2024/01.mkv"), None);
    }

    #[test]
    fn is_structural_dir_classifies_common_shapes() {
        for s in [
            "Season 1", "S01", "season", "Disc-2", "CD 3", "Specials", "OVA", "1", "BDMV",
        ] {
            assert!(is_structural_dir(s), "{s:?} should be structural");
        }
        for s in ["RahXephon", "Sousou no Frieren", "K-On!", "Re Zero"] {
            assert!(!is_structural_dir(s), "{s:?} should look like a title");
        }
    }

    /// #26: a name-matched file whose contents mismatch because it is
    /// still being written (an external download/copy landing in a media
    /// root) is re-checked on its own once its mtime/size hold still —
    /// resolving Verified within seconds, not at the next library scan
    /// a minute later.
    #[tokio::test]
    async fn mismatched_candidate_reresolves_after_the_write_quiesces() {
        let root = tempfile::tempdir().unwrap();
        let full = b"a complete episode file with real contents".as_slice();
        let hashed = ed2k_hash_bytes(full);
        // The file exists under the right name but is truncated mid-write.
        let path = root.path().join("ep1.mkv");
        std::fs::write(&path, &full[..8]).unwrap();

        let db = tempfile::tempdir().unwrap();
        let storage = Storage::open(&db.path().join("test.db")).unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );
        rig.commands
            .send(FileCommand::Resolve {
                file: hashed.root,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { resolution, .. } => {
                assert!(matches!(resolution, Resolution::HashMismatch(_)));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        // The "download" finishes. No new Resolve command is sent — the
        // actor must notice by itself once the file goes quiet.
        std::fs::write(&path, full).unwrap();
        loop {
            match next_output(&mut rig).await {
                FileOutput::Resolved {
                    file,
                    resolution: Resolution::Verified(_),
                } if file == hashed.root => break,
                FileOutput::Resolved { resolution, .. } => {
                    panic!("re-check produced {resolution:?}, expected Verified")
                }
                _ => continue,
            }
        }
    }

    /// #26 guard: a *stable* mismatch (a different encode, not a file
    /// mid-write) is never re-hashed — its cache row still matches its
    /// on-disk state, so there is nothing new to check.
    #[tokio::test]
    async fn stable_mismatch_is_not_rehashed() {
        let root = tempfile::tempdir().unwrap();
        let wanted = ed2k_hash_bytes(b"the version the playlist wants");
        let path = root.path().join("ep1.mkv");
        std::fs::write(&path, b"a different encode, complete and quiet").unwrap();

        let db = tempfile::tempdir().unwrap();
        let storage = Storage::open(&db.path().join("test.db")).unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );
        rig.commands
            .send(FileCommand::Resolve {
                file: wanted.root,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { resolution, .. } => {
                assert!(matches!(resolution, Resolution::HashMismatch(_)));
            }
            other => panic!("unexpected output: {other:?}"),
        }
        // Give the recheck poller ample time to (wrongly) fire.
        let extra = tokio::time::timeout(Duration::from_secs(4), rig.outputs.recv()).await;
        assert!(
            extra.is_err(),
            "a quiet, unchanged mismatch produced {extra:?}"
        );
    }

    /// #21: scan *hashing* defers while transfer traffic is active —
    /// indexing is bulk disk work with no deadline, transfers are
    /// latency-sensitive (a silent source is snubbed at 30s) — and
    /// resumes once transfers go quiet.
    #[tokio::test]
    async fn scan_hashing_defers_while_transfers_are_active() {
        let root = tempfile::tempdir().unwrap();
        // A servable, already-indexed file...
        let served = b"an episode being served to a peer".as_slice();
        let served_hash = ed2k_hash_bytes(served);
        let served_path = root.path().join("ep1.mkv");
        std::fs::write(&served_path, served).unwrap();
        // ...and a new file the scan will want to hash.
        let fresh = b"a brand new file for the scan".as_slice();
        let fresh_hash = ed2k_hash_bytes(fresh);
        std::fs::write(root.path().join("ep2.mkv"), fresh).unwrap();

        let db = tempfile::tempdir().unwrap();
        let storage = Storage::open(&db.path().join("test.db")).unwrap();
        // Pre-index ep1 so the walk's worklist is exactly ep2.
        let metadata = std::fs::metadata(&served_path).unwrap();
        storage
            .upsert_hash_cache(
                &served_path,
                mtime_millis(&metadata).unwrap(),
                &served_hash,
                1,
            )
            .unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );

        // Make ep1 servable (a Verified local copy)…
        rig.commands
            .send(FileCommand::Resolve {
                file: served_hash.root,
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
        // …and serve a chunk: transfer traffic is now active.
        let peer = dessplay_core::net::PeerId::from(dessplay_core::types::UserId::new("kim"));
        rig.commands
            .send(FileCommand::PeerMessage {
                from: peer,
                message: Box::new(dessplay_core::net::PeerMessage::ChunkRequest {
                    file: served_hash.root,
                    chunks: vec![0],
                }),
            })
            .await
            .unwrap();

        // Scan now. The walk runs (it's cheap), but ep2's hash defers.
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();
        let deferred_until = std::time::Instant::now() + Duration::from_millis(800);
        while std::time::Instant::now() < deferred_until {
            let Ok(Some(output)) =
                tokio::time::timeout(Duration::from_millis(100), rig.outputs.recv()).await
            else {
                continue;
            };
            if let FileOutput::LibraryIndexed { files } = &output {
                assert!(
                    !files.iter().any(|f| f.hash == fresh_hash.root),
                    "scan hashed ep2 while a transfer was active"
                );
            }
        }

        // Transfers stop; after the quiet window the hash runs.
        loop {
            match next_output(&mut rig).await {
                FileOutput::LibraryIndexed { files }
                    if files.iter().any(|f| f.hash == fresh_hash.root) =>
                {
                    break;
                }
                _ => continue,
            }
        }
    }
}
