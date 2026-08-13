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
use dessplay_core::net::framing::{read_frame, write_frame};
use dessplay_core::net::{BiStream, PeerId, PeerMessage, chunk_range};
use dessplay_core::types::{AniDbSeriesId, Ed2kHash, FileAvailability};
use dessplay_core::wire;
use tokio::sync::mpsc;

use crate::actors::network::Clock;
use crate::config::CacheRetention;
use crate::download::{DownloadAction, DownloadConfig, Downloads};
use crate::storage::{CacheEntry, SeriesKey, Storage, WatchRecord};
use crate::torrent::engine::{TorrentEngine, TorrentImportId};
use crate::torrent::nyaa::{self, NyaaBrowseResult, NyaaSource};

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

/// Visible stage of a user-selected Nyaa import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NyaaImportStage {
    /// Torrent pieces are arriving.
    Downloading,
    /// The complete payload is being assigned its ed2k identity.
    Hashing,
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
    /// Search Nyaa's anime category for safe single-file torrents.
    SearchNyaa {
        /// Free-form user query.
        query: String,
    },
    /// Download a selected Nyaa result, then hash and add it.
    StartNyaaImport {
        /// UI-generated local import identity.
        id: TorrentImportId,
        /// Inspected single-file result.
        result: NyaaBrowseResult,
        /// Playlist anchor captured when search opened.
        after: Option<Ed2kHash>,
    },
    /// Cancel and delete a pending Nyaa import.
    CancelNyaaImport {
        /// Pending import identity.
        id: TorrentImportId,
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
    /// (the first media root). The actor builds the destination, optionally
    /// including a series-name subdirectory, since it owns the media roots
    /// (design.md, Archive).
    Archive {
        /// The cached file.
        file: Ed2kHash,
        /// Series name for the subdirectory (synced metadata always has
        /// one); `None` falls back to an "Unsorted" folder.
        series_name: Option<String>,
        /// Original filename to archive under.
        filename: String,
        /// Whether to place the file in a series-name subdirectory.
        subdirectory: bool,
    },
    /// Eviction pass (startup and EOF-advance).
    RunEviction {
        /// Never evicted: now-playing and queued unwatched entries.
        protected: HashSet<Ed2kHash>,
        /// Group watched flags (an entry behind the group's progress
        /// is evictable even if never personally watched).
        group_watched: HashSet<Ed2kHash>,
        /// Every playlist entry's hash. A cached file *not* in this set
        /// is no longer referenced and is evictable regardless of watched.
        playlist: HashSet<Ed2kHash>,
    },
    /// Media roots changed (settings save).
    SetMediaRoots(Vec<PathBuf>),
    /// Retention policy changed (settings save).
    SetRetention(CacheRetention),
    /// The BitTorrent setting changed (settings save). Disabling applies
    /// immediately: seeding torrents are removed (files deleted) and
    /// pending Nyaa imports are cancelled. Enabling only works when the
    /// engine was constructed at startup; otherwise it stays a no-op
    /// until restart (the session posts the notice).
    SetTorrentEnabled(bool),
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
    /// A per-transfer data stream is live (see the network actor's
    /// `TransferStream` event). `outbound` = we opened it (a download
    /// we drive); otherwise a peer wants `file` from us (a serve
    /// request, answered by a dedicated serve task so one slow
    /// downloader never stalls another — the pre-v9 shared relay
    /// stream did exactly that).
    TransferStream {
        /// The peer at the other end.
        peer: dessplay_core::net::PeerId,
        /// The file the stream transfers.
        file: Ed2kHash,
        /// Whether we opened it (download) or they did (serve).
        outbound: bool,
        /// The live stream.
        stream: BiStream,
    },
    /// The network layer could not satisfy an [`FileOutput::OpenTransfer`]
    /// request (link down or backlogged, open or header write failed) —
    /// the failure half of the answered-request contract. Clears the
    /// pending-stream queue for `(peer, file)` and requeues the source's
    /// in-flight chunks, so the next download tick re-plans and
    /// re-requests a stream instead of waiting forever on a request the
    /// network dropped.
    TransferStreamFailed {
        /// The uploader the stream was for.
        peer: dessplay_core::net::PeerId,
        /// The file the stream was for.
        file: Ed2kHash,
    },
    /// The control connection died. The server closes a session's
    /// transfer connection when its control connection ends, so every
    /// data stream — including opens still waiting on an answer inside
    /// the (now torn-down) transfer link — is implicitly dead. Live
    /// streams' readers observe the close themselves (`DownloadClosed`);
    /// the never-opened pending queues have no reader, so this fails
    /// them all explicitly, keeping the answered-request contract
    /// airtight across a reconnect.
    TransferLinkReset,
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
    /// A user-initiated Nyaa search completed.
    NyaaSearchFinished {
        /// Echoed query, used to reject stale modal results.
        query: String,
        /// Safe single-file results or a request-level error.
        result: Result<Vec<NyaaBrowseResult>, String>,
    },
    /// Pending Nyaa import progress for the local overlay/active list.
    NyaaImportProgress {
        /// Local import identity.
        id: TorrentImportId,
        /// Payload filename.
        filename: String,
        /// Downloading or hashing.
        stage: NyaaImportStage,
        /// Completed bytes for the stage.
        done_bytes: u64,
        /// Total bytes for the stage.
        total_bytes: u64,
    },
    /// A pending import ended. Successful imports carry the discovered
    /// identity and become ordinary playlist entries in the session layer.
    NyaaImportFinished {
        /// Local import identity.
        id: TorrentImportId,
        /// Payload filename.
        filename: String,
        /// Playlist anchor captured at selection time.
        after: Option<Ed2kHash>,
        /// Discovered identity and local path, or failure/cancellation text.
        result: Result<(Ed2kFileHash, PathBuf), String>,
    },
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
    /// Relay a small file-transfer message to a peer (availability,
    /// block hashes, `CannotServe`). The bridge loop turns this into a
    /// `NetworkCommand::SendPeer`. Bulk chunk traffic never rides this
    /// path — it lives on per-transfer data streams.
    SendPeer {
        /// Destination peer.
        to: dessplay_core::net::PeerId,
        /// The message.
        message: Box<dessplay_core::net::PeerMessage>,
    },
    /// Ask the network layer for a data stream to `to` for `file` (we
    /// are downloading from them). The bridge loop turns this into a
    /// `NetworkCommand::OpenTransferStream`; the answer comes back as
    /// [`FileCommand::TransferStream`] with `outbound: true`, or as
    /// [`FileCommand::TransferStreamFailed`] — the network layer
    /// answers every request (the answered-request contract), because
    /// this actor's "already asked" latch is the pending queue itself.
    OpenTransfer {
        /// The uploader.
        to: dessplay_core::net::PeerId,
        /// The file.
        file: Ed2kHash,
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
    /// An in-flight peer download became playable: every chunk in the
    /// 20% window ahead of the playback anchor is verified. The session
    /// loads the partial file into the player (it assembles in place at
    /// the final cache path, so playback continues seamlessly into
    /// completion). Emitted on each false→true edge; the session's load
    /// is idempotent.
    DownloadPlayable {
        /// The file.
        file: Ed2kHash,
        /// The partial file in the cache (its final path).
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
    /// The BitTorrent engine for the explicit Nyaa browse import;
    /// `None` disables the torrent path entirely.
    pub torrent: Option<Arc<dyn TorrentEngine>>,
    /// The nyaa search backing the browse import; `None` disables it.
    pub nyaa: Option<Arc<dyn NyaaSource>>,
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
        /// Active roots for which none of the previously recorded files
        /// remains reachable.
        vanished_roots: Vec<PathBuf>,
        /// Active roots proven online by at least one recorded file.
        online_roots: Vec<PathBuf>,
    },
    /// One library-scan file finished hashing.
    LibraryHashed {
        /// The file that was hashed.
        item: ScanItem,
        /// Its hash, or why not.
        result: std::io::Result<Ed2kFileHash>,
    },
    /// User-initiated browse search completed.
    NyaaBrowseSearched {
        query: String,
        result: Result<Vec<NyaaBrowseResult>, String>,
    },
    /// A user-selected torrent payload finished ed2k hashing.
    NyaaImportHashed {
        id: TorrentImportId,
        payload: PathBuf,
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
    /// Media root that owns this indexed path.
    media_root: PathBuf,
}

/// Run the actor until the command channel closes.
pub async fn run(
    config: FileConfig,
    mut commands: mpsc::Receiver<FileCommand>,
    out: mpsc::Sender<FileOutput>,
) {
    let (done_tx, mut done_rx) = mpsc::channel::<Done>(64);
    // Data-stream traffic (chunk arrivals, stream/serve lifecycle).
    // Bounded: a fast stream reader blocks here while the actor writes
    // chunks to disk, which is exactly the backpressure that keeps a
    // reader from outrunning storage.
    let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(64);
    // Captured before `config` moves into the actor.
    let scan_interval = config.scan_interval;
    let scan_enabled = scan_interval.is_some();
    let mut actor = match Actor::new(config, out, done_tx, stream_tx) {
        Ok(actor) => actor,
        Err(e) => {
            tracing::error!("file actor failed to initialize: {e}");
            return;
        }
    };
    // Torrents never survive a restart: the previous run's leftover
    // payload dirs are garbage by construction. Swept before any
    // commands land.
    actor.sweep_torrents_dir();
    // Drives snub detection and pipeline refill.
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
            ev = stream_rx.recv() => {
                // Same lifetime argument as done_rx: the actor holds a
                // stream_tx clone.
                let Some(ev) = ev else { break };
                actor.on_stream_event(ev).await;
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
    /// The live BitTorrent gate: starts as `torrent.is_some()` and
    /// follows the setting via [`FileCommand::SetTorrentEnabled`].
    /// Disabling flips this off immediately; enabling only sticks when
    /// the engine exists (it is constructed at startup or never).
    torrent_enabled: bool,
    /// The BitTorrent engine (`None` = torrent path disabled).
    torrent: Option<Arc<dyn TorrentEngine>>,
    /// The nyaa browse search source (`None` = torrent path disabled).
    nyaa: Option<Arc<dyn NyaaSource>>,
    /// User-selected torrents that do not have an ed2k identity yet.
    nyaa_imports: HashMap<TorrentImportId, PendingNyaaImport>,
    /// Live download data streams we opened: (source, file) → write
    /// half (chunk requests / cancels) plus its reader task.
    download_streams: HashMap<(PeerId, Ed2kHash), DownloadStream>,
    /// Messages queued for a data stream we've asked for but not yet
    /// received; the first queued message triggers the `OpenTransfer`.
    /// Cleared when the stream arrives (flushed) or the download ends.
    pending_streams: HashMap<(PeerId, Ed2kHash), Vec<PeerMessage>>,
    /// Serve tasks, one per incoming data stream: (requester, file).
    /// Each owns its stream and paces itself against `upload`, so a
    /// slow downloader backpressures only its own stream. Dropping a
    /// guard aborts the task.
    serve_tasks: HashMap<(PeerId, Ed2kHash), TaskGuard>,
    /// Upload pacing for serving chunks (`None` = unlimited); shared
    /// across serve tasks so the cap covers their sum.
    upload: Arc<UploadPacer>,
    /// Last shared-clock millis we wrote a `Downloading` progress
    /// update, per file (≤1/s throttle).
    last_progress_at: HashMap<Ed2kHash, u64>,
    /// Last playable verdict written per file, so a flip bypasses the
    /// progress throttle (it gates the group) and the false→true edge
    /// hands the session the partial file to load.
    last_playable_written: HashMap<Ed2kHash, bool>,
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
    /// Files the session asked us to resolve that we could not verify
    /// locally (hash → playlist filename): NotFound or HashMismatch
    /// outcomes, cleared on Verified. When the library walk turns up a
    /// new file bearing one of these names, it is resolved immediately —
    /// outside the scan-hashing transfer deferral (#21) — so a copy that
    /// arrives through another channel (a bittorrent download racing the
    /// peer prefetch, 2026-07-03) is adopted and the peer download
    /// cancelled instead of running to completion or a restart.
    wanted: HashMap<Ed2kHash, String>,
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
    /// Feeds stream traffic (chunk arrivals, lifecycle) back into the
    /// actor loop from stream reader / serve tasks.
    stream_tx: mpsc::Sender<StreamEvent>,
}

/// Aborts a spawned task when dropped — ties stream reader and serve
/// tasks to their registry entries.
struct TaskGuard(tokio::task::JoinHandle<()>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A live download data stream: the write half (requests/cancels flow
/// down it) and the reader task feeding its chunks into the actor.
struct DownloadStream {
    send: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    _reader: TaskGuard,
}

/// Traffic from stream reader and serve tasks back into the actor loop.
enum StreamEvent {
    /// A chunk arrived on a download stream.
    Data {
        /// The source peer.
        from: PeerId,
        /// The file.
        file: Ed2kHash,
        /// Chunk index.
        index: u32,
        /// The chunk's bytes.
        data: Vec<u8>,
    },
    /// A download stream closed (uploader gone, link reset). The next
    /// send toward that source reopens one.
    DownloadClosed {
        /// The source peer.
        from: PeerId,
        /// The file.
        file: Ed2kHash,
    },
    /// A serve task exited (downloader closed the stream, or the file
    /// vanished under it).
    ServeEnded {
        /// The requester.
        to: PeerId,
        /// The file.
        file: Ed2kHash,
    },
}

struct PendingNyaaImport {
    result: NyaaBrowseResult,
    after: Option<Ed2kHash>,
    hashing: bool,
    last_progress_bytes: u64,
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

/// Orphaned cache files (hash-named, but with no `cache_entries` row)
/// older than this are swept at startup. A younger one may be an
/// in-flight or just-abandoned peer-download partial — `download_path`
/// is the final cache path, so an interrupted download leaves one — and
/// is left alone; the age cutoff keeps the sweep off anything recent.
const ORPHAN_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
/// Removed roots keep their index long enough for a quick remove/re-add.
const REMOVED_ROOT_GRACE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Put a verified torrent payload at the hash-addressed cache path:
/// hardlink (same filesystem by construction — both under the cache
/// dir), falling back to a copy. Any stale partial at the destination
/// is removed first. The payload stays in place, seeding.
fn place_in_cache(payload: &Path, cache_path: &Path) -> std::io::Result<()> {
    if let Err(e) = std::fs::remove_file(cache_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(e);
    }
    if std::fs::hard_link(payload, cache_path).is_ok() {
        return Ok(());
    }
    std::fs::copy(payload, cache_path).map(|_| ())
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
        stream_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<Self, crate::storage::StorageError> {
        let mut config = config;
        let started = std::time::Instant::now();
        let now = (config.clock)() as i64;
        let _ = config.storage.reconcile_library_roots(
            &config.media_roots,
            now,
            REMOVED_ROOT_GRACE.as_millis() as i64,
        )?;
        // v5 backfill and deterministic overlap ownership: the first
        // effective root containing a path owns it, matching walk order.
        for row in config.storage.hash_cache()? {
            if let Some(root) = config
                .media_roots
                .iter()
                .find(|root| row.path.starts_with(root))
            {
                config.storage.set_hash_cache_root(&row.path, root)?;
            }
        }
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
        let mut live_cache_paths: HashSet<PathBuf> = HashSet::new();
        for entry in config.storage.cache_entries().unwrap_or_default() {
            let live = std::fs::metadata(&entry.path)
                .map(|m| m.len() == entry.size_bytes)
                .unwrap_or(false);
            if live {
                live_cache_paths.insert(entry.path.clone());
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
        // Sweep orphaned cache files: hash-named files in the cache root
        // with no surviving `cache_entries` row. run_eviction only
        // iterates cache_entries, so these are invisible to it and leak
        // forever — the case a DB reset (bookkeeping gone, files stay)
        // or an abandoned peer-download partial produces. Delete only
        // those older than a week by mtime (see ORPHAN_MAX_AGE), leaving
        // anything recent that might still be wanted.
        let mut orphans_swept = 0usize;
        if let Ok(dir) = std::fs::read_dir(&config.cache_dir) {
            for dirent in dir.flatten() {
                let path = dirent.path();
                let hash_named = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.parse::<Ed2kHash>().is_ok());
                if !hash_named || live_cache_paths.contains(&path) {
                    continue;
                }
                let metadata = match std::fs::metadata(&path) {
                    Ok(m) if m.is_file() => m,
                    _ => continue,
                };
                let old_enough = mtime_millis(&metadata)
                    .is_some_and(|mt| now.saturating_sub(mt) >= ORPHAN_MAX_AGE.as_millis() as i64);
                if !old_enough {
                    continue;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        tracing::info!(
                            path = %path.display(),
                            bytes = metadata.len(),
                            "sweeping orphaned cache file (no bookkeeping)"
                        );
                        let _ = config.storage.remove_hash_cache(&path);
                        hash_cache.remove(&path);
                        orphans_swept += 1;
                    }
                    Err(e) => tracing::warn!(path = %path.display(), "sweeping orphan: {e}"),
                }
            }
        }
        tracing::debug!(
            cached_hashes = hash_cache.len(),
            manual_mappings = manual.len(),
            cache_reconciled = reconciled,
            cache_pruned = pruned,
            orphans_swept,
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
            torrent_enabled: config.torrent.is_some(),
            torrent: config.torrent,
            nyaa: config.nyaa,
            nyaa_imports: HashMap::new(),
            download_streams: HashMap::new(),
            pending_streams: HashMap::new(),
            serve_tasks: HashMap::new(),
            upload: Arc::new(UploadPacer::new(config.upload_limit)),
            last_progress_at: HashMap::new(),
            last_playable_written: HashMap::new(),
            scan_pending_commits: Vec::new(),
            scan_worklist: std::collections::VecDeque::new(),
            scan_hashing: false,
            scan_walking: false,
            scan_done: 0,
            scan_total: 0,
            scan_started: None,
            scan_failed: 0,
            scan_log_step: 1,
            wanted: HashMap::new(),
            rechecks: HashMap::new(),
            last_transfer_activity: None,
            scan_transfer_quiet: config.scan_transfer_quiet,
            scan_defer_logged: false,
            out,
            done_tx,
            stream_tx,
        })
    }

    async fn on_command(&mut self, cmd: FileCommand) {
        match cmd {
            FileCommand::Resolve { file, filename } => self.resolve(file, filename).await,
            FileCommand::HashAdd { path, after } => self.hash_add(path, after).await,
            FileCommand::SearchNyaa { query } => self.search_nyaa(query).await,
            FileCommand::StartNyaaImport { id, result, after } => {
                self.start_nyaa_import(id, result, after).await;
            }
            FileCommand::CancelNyaaImport { id } => {
                self.finish_nyaa_import(id, Err("Cancelled".to_string()))
                    .await;
            }
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
                subdirectory,
            } => {
                self.archive(file, series_name, filename, subdirectory)
                    .await
            }
            FileCommand::RunEviction {
                protected,
                group_watched,
                playlist,
            } => {
                self.run_eviction(&protected, &group_watched, &playlist)
                    .await
            }
            FileCommand::SetMediaRoots(roots) => {
                self.reconcile_media_roots(roots).await;
                // New roots may hold files we've never indexed.
                self.start_library_scan();
            }
            FileCommand::SetRetention(retention) => self.retention = retention,
            FileCommand::SetTorrentEnabled(enabled) => self.set_torrent_enabled(enabled).await,
            FileCommand::RescanLibrary => self.start_library_scan(),
            FileCommand::StartDownload {
                file,
                size_bytes,
                sources,
                play_chunk,
            } => {
                // A file we already hold never (re)starts a download. The
                // session emits StartDownload from its *own* resolution
                // map, which lags this actor's — the FileOutput::Resolved
                // recording a freshly-landed local copy may still be in
                // flight — so a snapshot processed in that window re-emits
                // for a file whose redundant download was just cancelled.
                // Without this guard that re-emit re-created the deleted
                // partial and re-downloaded the whole file.
                if self.local_files.contains_key(&file) {
                    tracing::debug!(%file, "ignoring StartDownload for a file we already hold");
                    return;
                }
                // An already-playable active download re-offers its
                // partial on every refresh: the playable *edge* may have
                // fired while the file wasn't now-playing yet (a
                // prefetch), and the session only loads the partial for
                // the current now-playing file. Re-emitting at the
                // session's own refresh cadence (~1/s) makes the load
                // catch up when now-playing advances onto the file; the
                // session's handler is idempotent.
                if self.downloads.is_active(&file)
                    && self.last_playable_written.get(&file) == Some(&true)
                {
                    let _ = self
                        .out
                        .send(FileOutput::DownloadPlayable {
                            file,
                            path: self.download_path(file),
                        })
                        .await;
                }
                self.start_peer_download(file, size_bytes, sources, play_chunk)
                    .await;
            }
            FileCommand::PeerMessage { from, message } => {
                self.on_peer_message(from, *message).await;
            }
            FileCommand::TransferStream {
                peer,
                file,
                outbound,
                stream,
            } => {
                if outbound {
                    self.on_download_stream(peer, file, stream).await;
                } else {
                    self.on_serve_stream(peer, file, stream);
                }
            }
            FileCommand::TransferStreamFailed { peer, file } => {
                self.on_transfer_stream_failed(peer, file);
            }
            FileCommand::TransferLinkReset => self.on_transfer_link_reset(),
            FileCommand::ForgetLocalFile { file } => self.lost_local_file(file).await,
        }
    }

    /// The network layer answered an [`FileOutput::OpenTransfer`] with
    /// failure: see [`FileCommand::TransferStreamFailed`]. Drops the
    /// queued sends (they will never be delivered) and requeues the
    /// source's in-flight chunks **without** an immediate re-plan — a
    /// failure can come back in the same breath while the link is down,
    /// and re-planning here would re-ask instantly and spin through the
    /// failure loop. The download tick (250ms) re-plans, re-requests,
    /// and thereby re-asks for a stream: a paced retry.
    fn on_transfer_stream_failed(&mut self, peer: PeerId, file: Ed2kHash) {
        let key = (peer.clone(), file);
        if self.download_streams.contains_key(&key) {
            // A live stream arrived since this failure was reported (a
            // stale answer from an earlier request): nothing to redo.
            return;
        }
        if self.pending_streams.remove(&key).is_some() {
            tracing::debug!(
                %peer, %file,
                "stream open failed; dropping queued sends for a paced retry"
            );
        }
        self.downloads.requeue_source(file, &peer, (self.clock)());
    }

    /// The control connection (and with it the whole transfer plane)
    /// died: see [`FileCommand::TransferLinkReset`]. Every pending
    /// (never-opened) stream is failed; live streams' readers observe
    /// the connection close themselves and report `DownloadClosed`.
    fn on_transfer_link_reset(&mut self) {
        let keys: Vec<(PeerId, Ed2kHash)> = self.pending_streams.keys().cloned().collect();
        if keys.is_empty() {
            return;
        }
        tracing::debug!(
            pending = keys.len(),
            "transfer link reset; failing pending stream opens"
        );
        for (peer, file) in keys {
            self.on_transfer_stream_failed(peer, file);
        }
    }

    /// Periodic maintenance: snub/refill the downloads, poll pending
    /// Nyaa imports.
    async fn on_tick(&mut self) {
        let actions = self.downloads.tick((self.clock)());
        self.run_download_actions(actions).await;
        self.poll_nyaa_imports().await;
        self.poll_rechecks().await;
        // Resume deferred scan hashing once transfers go quiet (#21).
        self.pump_library_scan();
    }

    async fn search_nyaa(&mut self, query: String) {
        let source = match self.nyaa.clone() {
            Some(source) if self.torrent_enabled => source,
            _ => {
                let _ = self
                    .out
                    .send(FileOutput::NyaaSearchFinished {
                        query,
                        result: Err(
                            "BitTorrent downloads are disabled; enable them in Settings."
                                .to_string(),
                        ),
                    })
                    .await;
                return;
            }
        };
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = nyaa::browse_single_file_results(source.as_ref(), &query, 20)
                .map_err(|e| e.to_string());
            let _ = done_tx.blocking_send(Done::NyaaBrowseSearched { query, result });
        });
    }

    async fn start_nyaa_import(
        &mut self,
        id: TorrentImportId,
        result: NyaaBrowseResult,
        after: Option<Ed2kHash>,
    ) {
        let engine = match self.torrent.clone() {
            Some(engine) if self.torrent_enabled => engine,
            _ => {
                let _ = self
                    .out
                    .send(FileOutput::NyaaImportFinished {
                        id,
                        filename: result.filename,
                        after,
                        result: Err(
                            "BitTorrent downloads are disabled; enable them in Settings."
                                .to_string(),
                        ),
                    })
                    .await;
                return;
            }
        };
        if self.nyaa_imports.contains_key(&id)
            || self
                .nyaa_imports
                .values()
                .any(|pending| pending.result.chosen.info_hash == result.chosen.info_hash)
        {
            let _ = self
                .out
                .send(FileOutput::NyaaImportFinished {
                    id,
                    filename: result.filename,
                    after,
                    result: Err("That torrent is already being added.".to_string()),
                })
                .await;
            return;
        }
        let filename = result.filename.clone();
        let size_bytes = result.size_bytes;
        engine.add_import(id, &result.chosen, self.nyaa_import_dir(id));
        self.nyaa_imports.insert(
            id,
            PendingNyaaImport {
                result,
                after,
                hashing: false,
                last_progress_bytes: 0,
            },
        );
        let _ = self
            .out
            .send(FileOutput::NyaaImportProgress {
                id,
                filename,
                stage: NyaaImportStage::Downloading,
                done_bytes: 0,
                total_bytes: size_bytes,
            })
            .await;
    }

    async fn poll_nyaa_imports(&mut self) {
        let Some(engine) = self.torrent.clone() else {
            return;
        };
        let ids: Vec<TorrentImportId> = self.nyaa_imports.keys().copied().collect();
        for id in ids {
            let Some(status) = engine.import_status(id) else {
                continue;
            };
            if status.error {
                self.finish_nyaa_import(id, Err("Torrent download failed.".to_string()))
                    .await;
                continue;
            }
            let Some(pending) = self.nyaa_imports.get_mut(&id) else {
                continue;
            };
            if status.finished {
                if pending.hashing {
                    continue;
                }
                let Some(payload) = status.payload else {
                    continue;
                };
                pending.hashing = true;
                let filename = pending.result.filename.clone();
                let total_bytes = pending.result.size_bytes;
                let _ = self
                    .out
                    .send(FileOutput::NyaaImportProgress {
                        id,
                        filename: filename.clone(),
                        stage: NyaaImportStage::Hashing,
                        done_bytes: 0,
                        total_bytes,
                    })
                    .await;
                let done_tx = self.done_tx.clone();
                let out = self.out.clone();
                tokio::task::spawn_blocking(move || {
                    let result = std::fs::File::open(&payload).and_then(|file| {
                        ed2k_hash_reader(NyaaImportProgressReader {
                            inner: file,
                            id,
                            filename,
                            total_bytes,
                            done_bytes: 0,
                            last_reported: 0,
                            events: out,
                        })
                    });
                    let _ = done_tx.blocking_send(Done::NyaaImportHashed {
                        id,
                        payload,
                        result,
                    });
                });
            } else if status.progress_bytes != pending.last_progress_bytes {
                pending.last_progress_bytes = status.progress_bytes;
                let _ = self
                    .out
                    .send(FileOutput::NyaaImportProgress {
                        id,
                        filename: pending.result.filename.clone(),
                        stage: NyaaImportStage::Downloading,
                        done_bytes: status.progress_bytes,
                        total_bytes: pending.result.size_bytes,
                    })
                    .await;
            }
        }
    }

    fn nyaa_import_dir(&self, id: TorrentImportId) -> PathBuf {
        self.cache_dir
            .join("torrents")
            .join(format!("import-{}", id.0))
    }

    async fn finish_nyaa_import(
        &mut self,
        id: TorrentImportId,
        result: Result<(Ed2kFileHash, PathBuf), String>,
    ) {
        let Some(pending) = self.nyaa_imports.remove(&id) else {
            return;
        };
        if result.is_err() {
            if let Some(engine) = &self.torrent {
                engine.remove_import(id, true);
            }
            let dir = self.nyaa_import_dir(id);
            if let Err(e) = std::fs::remove_dir_all(&dir)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %dir.display(), "removing cancelled import: {e}");
            }
        }
        let _ = self
            .out
            .send(FileOutput::NyaaImportFinished {
                id,
                filename: pending.result.filename,
                after: pending.after,
                result,
            })
            .await;
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
        self.expire_removed_roots();
        self.scan_walking = true;
        let roots = self.media_roots.clone();
        let cache = Arc::clone(&self.hash_cache);
        let done_tx = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let (hits, worklist, stale, vanished_roots, online_roots) =
                scan_library(&roots, &cache);
            tracing::debug!(
                hits = hits.len(),
                to_hash = worklist.len(),
                stale = stale.len(),
                vanished = vanished_roots.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "library walk finished"
            );
            let _ = done_tx.blocking_send(Done::LibraryWalk {
                hits,
                worklist,
                stale,
                vanished_roots,
                online_roots,
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

    /// Route an incoming peer message from the relay stream: block-hash
    /// solicitations are answered from our local copies; availability,
    /// hashes, and `CannotServe` feed the download scheduler. Chunk
    /// traffic never arrives here since protocol v9 — it lives on
    /// per-transfer data streams.
    async fn on_peer_message(&mut self, from: PeerId, message: PeerMessage) {
        // Any peer traffic — requests we serve, chunks we receive — is
        // transfer activity that defers scan hashing (#21).
        self.note_transfer_activity();
        match message {
            PeerMessage::BlockHashRequest { file } => self.serve_block_hashes(from, file).await,
            PeerMessage::ChunkRequest { .. } | PeerMessage::Cancel { .. } => {
                tracing::debug!(%from, "chunk control on the relay stream; ignoring (pre-v9?)");
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
                    // Chunk control rides the per-transfer data stream;
                    // everything else the relay stream.
                    match message {
                        PeerMessage::ChunkRequest { file, .. }
                        | PeerMessage::Cancel { file, .. } => {
                            self.send_on_stream(to, file, message).await;
                        }
                        other => {
                            let _ = self
                                .out
                                .send(FileOutput::SendPeer {
                                    to,
                                    message: Box::new(other),
                                })
                                .await;
                        }
                    }
                }
                DownloadAction::Progress {
                    file,
                    progress_bps,
                    playable,
                } => {
                    // Throttle progress writes to at most once a second
                    // (a fast download crosses a block — a progress
                    // step — several times a second). A *playable* flip
                    // bypasses the throttle: it gates the whole group,
                    // so it must reach the synced state at once.
                    let now = (self.clock)();
                    let flipped = self.last_playable_written.get(&file) != Some(&playable);
                    let due =
                        now.saturating_sub(*self.last_progress_at.get(&file).unwrap_or(&0)) >= 1000;
                    if flipped || due {
                        self.last_progress_at.insert(file, now);
                        self.last_playable_written.insert(file, playable);
                        let availability = if playable {
                            FileAvailability::DownloadingPlayable { progress_bps }
                        } else {
                            FileAvailability::Downloading { progress_bps }
                        };
                        let _ = self
                            .out
                            .send(FileOutput::Availability { file, availability })
                            .await;
                    }
                    // Newly playable: hand the session the partial file so
                    // it can load it into the player (design.md, File
                    // State — a Downloading file *plays* once the window
                    // ahead is verified, instead of leaving this user
                    // behind on a placeholder when the group unpauses).
                    if flipped && playable {
                        let _ = self
                            .out
                            .send(FileOutput::DownloadPlayable {
                                file,
                                path: self.download_path(file),
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
                    self.drop_download_streams(file);
                    self.last_progress_at.remove(&file);
                    self.last_playable_written.remove(&file);
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
        self.wanted.remove(&file);
        self.local_files.insert(file, path.clone());
        self.drop_download_streams(file);
        self.last_progress_at.remove(&file);
        self.last_playable_written.remove(&file);
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

    /// A complete, verified local copy of `file` exists at `path`: the
    /// single seam every "a local copy turned up" channel goes through —
    /// the resolve path, the library-scan adoption, and the browse-import
    /// completion — so no path can skip cancelling the now-redundant
    /// peer download again (the 2026-08-12 review found the browse
    /// import had regressed exactly that). The cancel deletes the
    /// partial at `<cache>/<hash>` and drops its streams, so a caller
    /// that then places a copy at that same path must call this
    /// *before* placing (see `on_nyaa_import_hashed`). The genuine
    /// download-completion path (`on_download_complete`) runs after the
    /// scheduler has already retired the download and its `path` *is*
    /// the download path, so it never cancels here.
    async fn adopt_local_copy(&mut self, file: Ed2kHash, path: PathBuf) {
        if self.downloads.is_active(&file) && path != self.download_path(file) {
            self.cancel_redundant_peer_download(file).await;
        }
        self.local_files.insert(file, path);
    }

    /// A verified local copy turned up while a download for the same
    /// file was in flight (it arrived through another channel): tell
    /// the sources to drop our in-flight chunk requests and remove the
    /// partial cache file. Callers go through [`Self::adopt_local_copy`];
    /// this is its cancel half.
    async fn cancel_redundant_peer_download(&mut self, file: Ed2kHash) {
        if self.downloads.is_active(&file) {
            tracing::info!(%file, "local copy appeared; cancelling the peer download");
        }
        let actions = self.downloads.cancel(&file);
        self.run_download_actions(actions).await;
        self.drop_download_streams(file);
        self.last_progress_at.remove(&file);
        self.last_playable_written.remove(&file);
        let partial = self.download_path(file);
        if let Err(e) = std::fs::remove_file(&partial)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %partial.display(), "removing partial download: {e}");
        }
    }

    /// Begin (or refresh) the peer-transfer download (Phase 9B).
    async fn start_peer_download(
        &mut self,
        file: Ed2kHash,
        size_bytes: u64,
        sources: Vec<PeerId>,
        play_chunk: u32,
    ) {
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

    /// Startup sweep of `<cache>/torrents/`: torrents never survive a
    /// restart (design.md, BitTorrent Downloads — an import seeds only
    /// for the session that downloaded it, and the engine keeps no
    /// persistent session), so everything left under the engine root —
    /// abandoned `import-*` payload dirs, prior versions' leftovers —
    /// is garbage. The one exception: an entry that still hosts a
    /// *registered cache file* (a completed import whose hardlink into
    /// the hash-addressed cache failed and was registered in place) is
    /// spared; deleting it would take the cached copy with it.
    fn sweep_torrents_dir(&self) {
        let torrents_dir = self.cache_dir.join("torrents");
        let Ok(dir) = std::fs::read_dir(&torrents_dir) else {
            return;
        };
        for dirent in dir.flatten() {
            let path = dirent.path();
            if self
                .local_files
                .values()
                .any(|local| local.starts_with(&path))
            {
                tracing::debug!(
                    path = %path.display(),
                    "sparing torrent dir hosting a registered cache file"
                );
                continue;
            }
            let result = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => tracing::info!(path = %path.display(), "sweeping stale torrent data"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(path = %path.display(), "sweeping torrent data: {e}"),
            }
        }
    }

    /// Apply a live BitTorrent-setting change (design.md, BitTorrent
    /// Downloads: disabling applies immediately). Disable: every
    /// seeding torrent (the uplink saturator) is removed with its
    /// files, and pending user-selected imports are cancelled. The
    /// cached copies of *completed* imports are untouched: they were
    /// hardlinked/copied into the hash-addressed cache at verification.
    /// The librqbit session (and its DHT socket) stays alive until
    /// restart — bounded chatter, no payload traffic.
    async fn set_torrent_enabled(&mut self, enabled: bool) {
        if enabled {
            self.torrent_enabled = self.torrent.is_some();
            tracing::info!(
                effective = self.torrent_enabled,
                "BitTorrent enabled{}",
                if self.torrent.is_some() {
                    ""
                } else {
                    " but no engine was started; restart to apply"
                }
            );
            return;
        }
        if !self.torrent_enabled {
            return;
        }
        self.torrent_enabled = false;
        tracing::info!("BitTorrent disabled; removing torrents");
        if let Some(engine) = self.torrent.clone() {
            for file in engine.active() {
                self.drop_torrent(file);
            }
        }
        let pending: Vec<TorrentImportId> = self.nyaa_imports.keys().copied().collect();
        for id in pending {
            self.finish_nyaa_import(id, Err("BitTorrent was disabled.".to_string()))
                .await;
        }
    }

    /// Stop seeding `file` (a promoted import): remove its torrent from
    /// the engine along with its payload files. The cached copy is a
    /// separate hardlink and survives; an emptied import dir is swept
    /// at the next startup. Safe to call when no torrent exists.
    fn drop_torrent(&mut self, file: Ed2kHash) {
        if let Some(engine) = &self.torrent {
            engine.remove(file, true);
        }
    }

    async fn on_nyaa_import_hashed(
        &mut self,
        id: TorrentImportId,
        payload: PathBuf,
        result: std::io::Result<Ed2kFileHash>,
    ) {
        let Some(pending) = self.nyaa_imports.get(&id) else {
            return;
        };
        let expected_size = pending.result.size_bytes;
        let hashed = match result {
            Ok(hashed) if hashed.size_bytes == expected_size => hashed,
            Ok(hashed) => {
                self.finish_nyaa_import(
                    id,
                    Err(format!(
                        "Torrent payload size changed (expected {expected_size}, got {}).",
                        hashed.size_bytes
                    )),
                )
                .await;
                return;
            }
            Err(e) => {
                self.finish_nyaa_import(id, Err(format!("Hashing downloaded file failed: {e}")))
                    .await;
                return;
            }
        };
        let file = hashed.root;
        // Re-key the import to its file so eviction and the live-disable
        // path can end its seeding by hash. Session-only: nothing is
        // persisted, and the torrent dies with the process.
        if let Some(engine) = &self.torrent {
            engine.promote_import(id, file);
        }
        // Already held at a real (non-cache) path — a library file the
        // user happened to also import: finish against that copy instead
        // of demoting it to a retention-evictable cache row that would
        // shadow it (2026-08-12 review, still-open). The torrent keeps
        // seeding from its import dir either way.
        let held = self
            .local_files
            .get(&file)
            .filter(|p| **p != self.download_path(file) && p.is_file())
            .cloned();
        if let Some(existing) = held {
            tracing::info!(
                %file,
                path = %existing.display(),
                "browse import matches a file we already hold; keeping the library copy"
            );
            self.adopt_local_copy(file, existing.clone()).await;
            self.finish_nyaa_import(id, Ok((hashed, existing))).await;
            return;
        }
        // The payload is a complete verified copy: adopt it, cancelling
        // any in-flight peer download of the same file *before* the
        // cache placement below — both share `<cache>/<hash>`, and
        // placing first would unlink the partial under the live
        // ChunkStore's fd while the orphaned download flapped the
        // group's gate (2026-08-12 review, regression).
        self.adopt_local_copy(file, payload.clone()).await;
        let cache_path = self.download_path(file);
        let local_path = match place_in_cache(&payload, &cache_path) {
            Ok(()) => cache_path,
            Err(e) => {
                tracing::warn!(%file, "placing Nyaa import in cache failed ({e}); using it in place");
                payload
            }
        };
        self.on_download_complete(file, local_path.clone(), hashed.blocks.clone())
            .await;
        self.finish_nyaa_import(id, Ok((hashed, local_path))).await;
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
            // serve those block hashes under this file's identity — and say
            // so: this is a *definitive* mismatch (unlike the uncached case
            // above, which may be a hash still in flight), so tell the
            // requester to stop soliciting us instead of leaving it to
            // re-ask a permanently-silent holder on every cooldown.
            tracing::debug!(%file, actual = %hashed.root,
                "cached hashes don't match the requested file; replying CannotServe");
            let _ = self
                .out
                .send(FileOutput::SendPeer {
                    to,
                    message: Box::new(PeerMessage::CannotServe { file }),
                })
                .await;
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
        // Anyone we were serving it to loses their stream (the guard's
        // Drop aborts the task; the downloader sees the close and moves
        // to another source).
        self.serve_tasks.retain(|(_, f), _| *f != file);
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
            // A torrent seeding this payload has lost its file too.
            self.drop_torrent(file);
        }
        let _ = self
            .out
            .send(FileOutput::Availability {
                file,
                availability: FileAvailability::Missing,
            })
            .await;
    }

    // ---- Per-transfer data streams (protocol v9).
    //
    // Chunk traffic lives on one QUIC stream per (peer, file), pumped
    // byte-for-byte through the server, so pacing is BBR + stream
    // backpressure end to end and one slow downloader never stalls
    // another (the pre-v9 shared relay stream and actor-loop serve
    // queue both did).

    /// Send a chunk-control message toward a source, on its data stream
    /// — opening one first if needed (messages queue until it arrives).
    async fn send_on_stream(&mut self, to: PeerId, file: Ed2kHash, message: PeerMessage) {
        let key = (to.clone(), file);
        if let Some(stream) = self.download_streams.get_mut(&key) {
            let Ok(frame) = wire::encode(&message) else {
                return;
            };
            // Control frames are tiny; QUIC/pump buffers absorb them,
            // so this await never meaningfully blocks the actor.
            if let Err(e) = write_frame(&mut stream.send, &frame).await {
                tracing::debug!(%to, %file, "data stream write failed: {e}");
                self.download_streams.remove(&key);
                self.queue_for_stream(to, file, message).await;
            }
            return;
        }
        self.queue_for_stream(to, file, message).await;
    }

    /// Queue a message for a not-yet-open data stream; the first queued
    /// message asks the network layer to open one. The queue-emptiness
    /// latch is sound because the network layer answers every open
    /// (stream or [`FileCommand::TransferStreamFailed`], which clears
    /// the queue) — the cap below is a belt-and-braces memory bound,
    /// not the recovery mechanism.
    async fn queue_for_stream(&mut self, to: PeerId, file: Ed2kHash, message: PeerMessage) {
        let queue = self.pending_streams.entry((to.clone(), file)).or_default();
        let fresh = queue.is_empty();
        queue.push(message);
        if queue.len() > PENDING_STREAM_MESSAGES {
            // Stale chunk control is worthless — the scheduler re-plans
            // everything the moment the stream (or its failure) lands.
            tracing::debug!(%to, %file, "pending stream queue full; dropping oldest");
            queue.remove(0);
        }
        if fresh {
            let _ = self.out.send(FileOutput::OpenTransfer { to, file }).await;
        }
    }

    /// A data stream we asked for is live: spawn its reader and flush
    /// whatever queued while it was being opened.
    async fn on_download_stream(&mut self, peer: PeerId, file: Ed2kHash, stream: BiStream) {
        if !self.downloads.is_active(&file) {
            // The download ended while the stream was in flight; drop it
            // (closing tells the uploader).
            self.pending_streams.remove(&(peer, file));
            return;
        }
        let BiStream { send, recv } = stream;
        let reader = tokio::spawn(read_download_stream(
            recv,
            peer.clone(),
            file,
            self.stream_tx.clone(),
        ));
        self.download_streams.insert(
            (peer.clone(), file),
            DownloadStream {
                send,
                _reader: TaskGuard(reader),
            },
        );
        if let Some(queue) = self.pending_streams.remove(&(peer.clone(), file)) {
            for message in queue {
                self.send_on_stream(peer.clone(), file, message).await;
            }
        }
    }

    /// A peer opened a data stream toward us: they want `file`. Spawn a
    /// dedicated serve task — it owns the stream, paces itself against
    /// the shared upload budget, and backpressures only itself.
    fn on_serve_stream(&mut self, peer: PeerId, file: Ed2kHash, stream: BiStream) {
        self.note_transfer_activity();
        let Some(path) = self.local_files.get(&file).cloned() else {
            // We advertised Ready but no longer hold it — same silent
            // "advertised but can't serve" failure as serve_block_hashes;
            // dropping the stream is the signal.
            tracing::debug!(%peer, %file, "serve stream for a file we don't hold; dropping");
            return;
        };
        tracing::debug!(%peer, %file, "serve stream opened");
        let task = tokio::spawn(serve_transfer(
            stream,
            peer.clone(),
            file,
            path,
            Arc::clone(&self.upload),
            Arc::clone(&self.clock),
            self.stream_tx.clone(),
        ));
        // A fresh stream for the same transfer replaces (aborts) any
        // stale predecessor.
        self.serve_tasks.insert((peer, file), TaskGuard(task));
    }

    /// Traffic and lifecycle from stream reader / serve tasks.
    async fn on_stream_event(&mut self, ev: StreamEvent) {
        match ev {
            StreamEvent::Data {
                from,
                file,
                index,
                data,
            } => {
                self.note_transfer_activity();
                let actions = self.downloads.on_peer_message(
                    from,
                    PeerMessage::ChunkData { file, index, data },
                    (self.clock)(),
                );
                self.run_download_actions(actions).await;
            }
            StreamEvent::DownloadClosed { from, file } => {
                tracing::debug!(%from, %file, "download stream closed");
                self.download_streams.remove(&(from.clone(), file));
                // A closed/reset stream is the snub signal (proposal
                // §3): requeue the source's in-flight chunks and
                // re-plan now — the next request toward it opens a
                // fresh stream — instead of leaving the window dead
                // until the 30s snub timeout.
                let actions = self
                    .downloads
                    .on_source_stream_lost(file, &from, (self.clock)());
                self.run_download_actions(actions).await;
            }
            StreamEvent::ServeEnded { to, file } => {
                tracing::debug!(%to, %file, "serve stream ended");
                self.serve_tasks.remove(&(to, file));
            }
        }
    }

    /// Drop every data stream and queued send for `file` — the download
    /// is over (complete, cancelled, or abandoned). Closing the streams
    /// is what tells the uploaders.
    fn drop_download_streams(&mut self, file: Ed2kHash) {
        self.download_streams.retain(|(_, f), _| *f != file);
        self.pending_streams.retain(|(_, f), _| *f != file);
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
                // Track unmet resolves so the library walk can spot their
                // files arriving through another channel; a verified copy
                // that isn't the download's own completed cache file makes
                // the peer download redundant — cancel it.
                match &resolution {
                    Resolution::Verified(_) => {
                        self.wanted.remove(&file);
                    }
                    Resolution::NotFound | Resolution::HashMismatch(_) => {
                        self.wanted.insert(file, filename.clone());
                    }
                }
                // A verified local copy can be served to peers.
                if let Resolution::Verified(path) = &resolution {
                    self.adopt_local_copy(file, path.clone()).await;
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
            Done::NyaaBrowseSearched { query, result } => {
                let _ = self
                    .out
                    .send(FileOutput::NyaaSearchFinished { query, result })
                    .await;
            }
            Done::NyaaImportHashed {
                id,
                payload,
                result,
            } => self.on_nyaa_import_hashed(id, payload, result).await,
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
                vanished_roots,
                online_roots,
            } => {
                self.scan_walking = false;
                self.reconcile_scan_roots(vanished_roots, online_roots, stale)
                    .await;
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
                // A new/changed file bearing the name of an unmet resolve
                // is worth verifying right now: resolve hashes it outside
                // the scan deferral (#21), so a copy that landed through
                // another channel is adopted even while its own peer
                // download keeps hashing deferred (2026-07-03). A copy
                // still being written resolves HashMismatch and the
                // quiescence watch (#26) picks it up from there — so skip
                // files already under watch.
                let arrived: Vec<(Ed2kHash, String)> = self
                    .scan_worklist
                    .iter()
                    .filter_map(|item| {
                        self.wanted
                            .iter()
                            .find(|(hash, name)| {
                                *name == &item.filename && !self.rechecks.contains_key(hash)
                            })
                            .map(|(hash, name)| (*hash, name.clone()))
                    })
                    .collect();
                for (hash, filename) in arrived {
                    tracing::info!(
                        file = %hash,
                        filename,
                        "library walk found a file we were missing; resolving now"
                    );
                    self.resolve(hash, filename).await;
                }
                if self.scan_total > 0 {
                    tracing::info!(
                        to_hash = self.scan_total,
                        files = ?self
                            .scan_worklist
                            .iter()
                            .map(|item| &item.path)
                            .collect::<Vec<_>>(),
                        "indexing media library"
                    );
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
                        self.queue_scan_hash_commit(
                            item.path.clone(),
                            item.mtime,
                            hashed,
                            &item.media_root,
                        );
                        // The scan hashed a file whose *contents* match an
                        // unmet resolve or an active download: adopt it.
                        // By-hash, so it works even under a different
                        // filename (where the walk trigger can't see it).
                        if self.wanted.remove(&root).is_some() || self.downloads.is_active(&root) {
                            tracing::info!(
                                file = %root,
                                path = %item.path.display(),
                                "library scan found a file we were missing; adopting"
                            );
                            self.rechecks.remove(&root);
                            self.adopt_local_copy(root, item.path.clone()).await;
                            let _ = self
                                .out
                                .send(FileOutput::Resolved {
                                    file: root,
                                    resolution: Resolution::Verified(item.path.clone()),
                                })
                                .await;
                        }
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
    fn prune_stale_index(&mut self, stale: Vec<PathBuf>) -> Vec<Ed2kHash> {
        if stale.is_empty() {
            return Vec::new();
        }
        let mut cache = (*self.hash_cache).clone();
        let mut lost = Vec::new();
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
                    lost.push(hashed.root);
                }
            }
        }
        self.hash_cache = Arc::new(cache);
        lost
    }

    /// Apply the root-level disappearance heuristic after a walk.
    async fn reconcile_scan_roots(
        &mut self,
        vanished: Vec<PathBuf>,
        online: Vec<PathBuf>,
        stale: Vec<PathBuf>,
    ) {
        let now = (self.clock)() as i64;
        for root in &online {
            if let Err(e) = self.storage.set_library_root_vanished(root, None) {
                tracing::error!(root = %root.display(), "clearing vanished root: {e}");
            }
        }
        let mut lost = self.prune_stale_index(stale);
        for root in &vanished {
            if let Err(e) = self.storage.set_library_root_vanished(root, Some(now)) {
                tracing::error!(root = %root.display(), "marking vanished root: {e}");
            }
            tracing::warn!(root = %root.display(), "all indexed files vanished; retaining index");
            let held: Vec<Ed2kHash> = self
                .local_files
                .iter()
                .filter(|(_, path)| path.starts_with(root))
                .map(|(hash, _)| *hash)
                .collect();
            for hash in held {
                self.local_files.remove(&hash);
                lost.push(hash);
            }
        }
        lost.sort_unstable();
        lost.dedup();
        for file in lost {
            let _ = self
                .out
                .send(FileOutput::Availability {
                    file,
                    availability: FileAvailability::Missing,
                })
                .await;
        }
    }

    async fn reconcile_media_roots(&mut self, roots: Vec<PathBuf>) {
        let removed: Vec<PathBuf> = self
            .media_roots
            .iter()
            .filter(|old| !roots.contains(old))
            .cloned()
            .collect();
        let now = (self.clock)() as i64;
        if let Err(e) =
            self.storage
                .reconcile_library_roots(&roots, now, REMOVED_ROOT_GRACE.as_millis() as i64)
        {
            tracing::error!("reconciling media roots: {e}");
        }
        for row in self.storage.hash_cache().unwrap_or_default() {
            if let Some(root) = roots.iter().find(|root| row.path.starts_with(root))
                && let Err(e) = self.storage.set_hash_cache_root(&row.path, root)
            {
                tracing::error!(path = %row.path.display(), "associating media root: {e}");
            }
        }
        self.media_roots = roots;
        let lost: Vec<Ed2kHash> = self
            .local_files
            .iter()
            .filter(|(_, path)| removed.iter().any(|root| path.starts_with(root)))
            .map(|(hash, _)| *hash)
            .collect();
        for file in lost {
            self.local_files.remove(&file);
            let _ = self
                .out
                .send(FileOutput::Availability {
                    file,
                    availability: FileAvailability::Missing,
                })
                .await;
        }
    }

    fn expire_removed_roots(&mut self) {
        let now = (self.clock)() as i64;
        let purged = match self.storage.reconcile_library_roots(
            &self.media_roots,
            now,
            REMOVED_ROOT_GRACE.as_millis() as i64,
        ) {
            Ok(roots) => roots,
            Err(e) => {
                tracing::error!("expiring removed media roots: {e}");
                return;
            }
        };
        if purged.is_empty() {
            return;
        }
        let mut cache = (*self.hash_cache).clone();
        cache.retain(|path, _| !purged.iter().any(|root| path.starts_with(root)));
        self.hash_cache = Arc::new(cache);
        for root in purged {
            tracing::info!(root = %root.display(), "purged removed media-root index after grace period");
        }
    }

    /// Persist one library-scan hash result to SQLite immediately, but
    /// only fold it into the in-memory `hash_cache` once
    /// [`SCAN_COMMIT_BATCH`] results have piled up (or
    /// [`Self::flush_scan_hash_commits`] is called explicitly, at scan
    /// completion) -- see [`SCAN_COMMIT_BATCH`] for why.
    fn queue_scan_hash_commit(
        &mut self,
        path: PathBuf,
        mtime: i64,
        hash: Ed2kFileHash,
        media_root: &Path,
    ) {
        let now = (self.clock)() as i64;
        if let Err(e) = self.storage.upsert_hash_cache(&path, mtime, &hash, now) {
            tracing::error!("persisting hash cache: {e}");
        }
        if let Err(e) = self.storage.set_hash_cache_root(&path, media_root) {
            tracing::error!("associating hash cache with media root: {e}");
        }
        if let Err(e) = self.storage.set_library_root_vanished(media_root, None) {
            tracing::error!("reactivating media root after hash: {e}");
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
            if let Some(root) = self.media_roots.iter().find(|root| path.starts_with(root))
                && let Err(e) = self.storage.set_hash_cache_root(&path, root)
            {
                tracing::error!("associating hash cache with media root: {e}");
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
        // a by-hash candidate (the filename search can't find it). Never
        // while a download for the file is running: the candidate would
        // be the live partial — a full ed2k pass over a moving, full-size
        // sparse file on every resolve (its mtime churn defeats the hash
        // cache), and a partial can never verify anyway.
        let cache_candidate = (!self.downloads.is_active(&file)).then(|| self.download_path(file));
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

    async fn archive(
        &mut self,
        file: Ed2kHash,
        series_name: Option<String>,
        filename: String,
        subdirectory: bool,
    ) {
        let result = self.archive_inner(file, series_name, &filename, subdirectory);
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
        subdirectory: bool,
    ) -> Result<PathBuf, String> {
        let download_root = self
            .media_roots
            .first()
            .ok_or("no download root configured")?;
        let filename = sanitize_component(filename);
        let dest = if subdirectory {
            // AniDB models each season as its own anime, so a single series
            // name is effectively one season's folder.
            let folder = sanitize_component(series_name.as_deref().unwrap_or("Unsorted"));
            download_root.join(folder).join(filename)
        } else {
            download_root.join(filename)
        };
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
        playlist: &HashSet<Ed2kHash>,
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
                playlist.contains(&entry.hash),
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
            // Seeding ends at eviction: remove the torrent (and its
            // payload — the cache path above may be just a hardlink to
            // it) alongside the cache entry. No-ops for peer downloads.
            self.drop_torrent(entry.hash);
            evicted.push(entry.hash);
        }
        if !evicted.is_empty() {
            let _ = self.out.send(FileOutput::Evicted { files: evicted }).await;
        }
    }
}

/// The eviction rule (design.md, Download Cache and Retention): a
/// cached file is evicted iff it is disposable — either watched
/// (personally, or behind the group) *or* no longer referenced by the
/// playlist at all — and not protected (now-playing / queued unwatched),
/// and its last access is older than the retention window. A file still
/// in the playlist but unwatched is kept (it is also `protected`, so this
/// is belt-and-suspenders); an abandoned download that has left the
/// playlist is reclaimed even though nobody ever watched it.
pub fn evictable(
    now: i64,
    retention: CacheRetention,
    entry: &CacheEntry,
    watched: bool,
    in_playlist: bool,
    protected: bool,
) -> bool {
    if protected || (in_playlist && !watched) {
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

/// How often the actor runs snub/refill maintenance (a safety net; data
/// arrival drives refill directly).
const DOWNLOAD_TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// Cap on messages queued for a data stream that hasn't arrived yet
/// (per `(peer, file)`). The answered-request contract means the queue
/// is short-lived; this bounds memory if an answer is ever lost anyway.
const PENDING_STREAM_MESSAGES: usize = 64;

/// Read a download data stream: each `ChunkData` frame feeds the actor
/// (the bounded stream-event channel is the disk-write backpressure);
/// any error or close reports `DownloadClosed` and exits.
async fn read_download_stream(
    mut recv: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    from: PeerId,
    file: Ed2kHash,
    tx: mpsc::Sender<StreamEvent>,
) {
    loop {
        let frame = match read_frame(&mut recv).await {
            Ok(frame) => frame,
            Err(_) => break,
        };
        match wire::decode::<PeerMessage>(&frame) {
            Ok(PeerMessage::ChunkData {
                file: f,
                index,
                data,
            }) if f == file => {
                let event = StreamEvent::Data {
                    from: from.clone(),
                    file,
                    index,
                    data,
                };
                if tx.send(event).await.is_err() {
                    return; // actor gone
                }
            }
            Ok(other) => {
                tracing::debug!(%from, %file, message = ?other, "unexpected data-stream frame");
            }
            Err(e) => {
                tracing::warn!(%from, %file, "undecodable data-stream frame: {e}");
                break;
            }
        }
    }
    let _ = tx.send(StreamEvent::DownloadClosed { from, file }).await;
}

/// Serve one transfer: read `ChunkRequest`/`Cancel` frames off the
/// stream into a queue, and stream `ChunkData` back as fast as the
/// stream accepts — the write await *is* the flow control (BBR and the
/// downloader's read pace bound it end to end), plus the shared upload
/// budget. Request order is serve order (the downloader front-loads its
/// sequential window). Exits when the stream closes or the file
/// vanishes; either way the closed stream is the downloader's signal.
async fn serve_transfer(
    stream: BiStream,
    to: PeerId,
    file: Ed2kHash,
    path: PathBuf,
    upload: Arc<UploadPacer>,
    clock: Clock,
    tx: mpsc::Sender<StreamEvent>,
) {
    let BiStream { mut send, recv } = stream;
    // Control frames read apart from the writer (read_frame is not
    // cancel-safe); unbounded is fine — requests are tiny and bounded
    // by the downloader's request window.
    let (ctl_tx, mut ctl) = mpsc::unbounded_channel::<PeerMessage>();
    let _reader = TaskGuard(tokio::spawn(async move {
        let mut recv = recv;
        while let Ok(frame) = read_frame(&mut recv).await {
            match wire::decode::<PeerMessage>(&frame) {
                Ok(msg) => {
                    if ctl_tx.send(msg).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    tracing::debug!("undecodable serve-stream frame: {e}");
                    return;
                }
            }
        }
    }));
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    let mut queued: HashSet<u32> = HashSet::new();
    let mut served: u64 = 0;
    let apply = |queue: &mut std::collections::VecDeque<u32>,
                 queued: &mut HashSet<u32>,
                 msg: PeerMessage| {
        match msg {
            PeerMessage::ChunkRequest { file: f, chunks } if f == file => {
                for chunk in chunks {
                    if queued.insert(chunk) {
                        queue.push_back(chunk);
                    }
                }
            }
            PeerMessage::Cancel { file: f, chunks } if f == file => {
                let cancelled: HashSet<u32> = chunks.into_iter().collect();
                queue.retain(|c| !cancelled.contains(c));
                queued.retain(|c| !cancelled.contains(c));
            }
            other => tracing::debug!(message = ?other, "unexpected serve-stream message"),
        }
    };
    'serve: loop {
        // Idle: wait for work (or the close, which drops ctl_tx).
        if queue.is_empty() {
            match ctl.recv().await {
                Some(msg) => apply(&mut queue, &mut queued, msg),
                None => break 'serve,
            }
        }
        // Absorb whatever else arrived (late cancels especially) before
        // committing to the next read+write.
        while let Ok(msg) = ctl.try_recv() {
            apply(&mut queue, &mut queued, msg);
        }
        let Some(index) = queue.pop_front() else {
            continue;
        };
        queued.remove(&index);
        let range = chunk_range(index, size);
        if range.is_empty() {
            continue;
        }
        upload.take(range.end - range.start, &clock).await;
        let data = match read_range(&path, range) {
            Ok(data) => data,
            Err(e) => {
                // Gone or truncated under us; the closed stream tells
                // the downloader, the actor's scan/guards re-resolve.
                tracing::debug!(%file, index, "serving chunk failed: {e}");
                break 'serve;
            }
        };
        served += data.len() as u64;
        let message = PeerMessage::ChunkData { file, index, data };
        let Ok(frame) = wire::encode(&message) else {
            break 'serve;
        };
        if write_frame(&mut send, &frame).await.is_err() {
            break 'serve;
        }
    }
    tracing::debug!(%to, %file, served_bytes = served, "serve task exiting");
    let _ = tx.send(StreamEvent::ServeEnded { to, file }).await;
}

/// The shared, task-safe face of [`UploadLimiter`]: serve tasks call
/// [`UploadPacer::take`], which waits out the budget instead of
/// returning `false`. A `std` mutex — held only for the arithmetic.
struct UploadPacer {
    inner: std::sync::Mutex<UploadLimiter>,
}

impl UploadPacer {
    fn new(limit: Option<u64>) -> Self {
        UploadPacer {
            inner: std::sync::Mutex::new(UploadLimiter::new(limit)),
        }
    }

    /// Wait until `bytes` fits the shared budget, then spend it.
    async fn take(&self, bytes: u64, clock: &Clock) {
        loop {
            let granted = match self.inner.lock() {
                Ok(mut limiter) => limiter.try_take(bytes, clock()),
                Err(_) => true, // poisoned: fail open, never wedge serving
            };
            if granted {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

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
type LibraryScan = (
    Vec<IndexedFile>,
    std::collections::VecDeque<ScanItem>,
    Vec<PathBuf>,
    Vec<PathBuf>,
    Vec<PathBuf>,
);

fn scan_library(roots: &[PathBuf], cache: &HashMap<PathBuf, (i64, Ed2kFileHash)>) -> LibraryScan {
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
                media_root: root.to_path_buf(),
            }),
        }
    });
    // Classify disappearance per root. If even one previously recorded
    // file is still a regular file, the root is online and independently
    // missing rows are genuine deletions. If none survive, retain the whole
    // cohort: removable storage commonly leaves an empty mountpoint behind.
    let mut stale = Vec::new();
    let mut vanished_roots = Vec::new();
    let mut online_roots = Vec::new();
    for root in roots {
        let recorded: Vec<&PathBuf> = cache
            .keys()
            .filter(|path| roots.iter().find(|candidate| path.starts_with(candidate)) == Some(root))
            .collect();
        let live: Vec<bool> = recorded.iter().map(|path| path.is_file()).collect();
        if root_disposition(&live) != RootDisposition::Vanished {
            online_roots.push(root.clone());
            stale.extend(
                recorded
                    .into_iter()
                    .zip(live)
                    .filter(|(path, is_live)| !seen.contains(*path) && !is_live)
                    .map(|(path, _)| path.clone()),
            );
        } else {
            vanished_roots.push(root.clone());
        }
    }
    (hits, worklist, stale, vanished_roots, online_roots)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootDisposition {
    Empty,
    Online,
    Vanished,
}

/// Pure policy seam for root-wide disappearance. An empty, never-indexed
/// root is online; any surviving recorded file proves individual absences
/// are deletions; only a non-empty cohort with zero survivors vanishes.
fn root_disposition(recorded_file_exists: &[bool]) -> RootDisposition {
    if recorded_file_exists.is_empty() {
        RootDisposition::Empty
    } else if recorded_file_exists.iter().any(|exists| *exists) {
        RootDisposition::Online
    } else {
        RootDisposition::Vanished
    }
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

/// Hash-progress reader for a pending Nyaa import.
struct NyaaImportProgressReader<R> {
    inner: R,
    id: TorrentImportId,
    filename: String,
    total_bytes: u64,
    done_bytes: u64,
    last_reported: u64,
    events: mpsc::Sender<FileOutput>,
}

impl<R: std::io::Read> std::io::Read for NyaaImportProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done_bytes += n as u64;
        if self.done_bytes - self.last_reported >= HASH_PROGRESS_STRIDE {
            self.last_reported = self.done_bytes;
            let _ = self.events.try_send(FileOutput::NyaaImportProgress {
                id: self.id,
                filename: self.filename.clone(),
                stage: NyaaImportStage::Hashing,
                done_bytes: self.done_bytes,
                total_bytes: self.total_bytes,
            });
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::time::Duration;

    use dessplay_core::hash::ed2k_hash_bytes;
    use proptest::prelude::*;

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

        // Args: (now, retention, entry, watched, in_playlist, protected).
        for retention in [
            CacheRetention::AfterWatch,
            CacheRetention::Keep(week),
            CacheRetention::Infinite,
        ] {
            // In the playlist and unwatched: kept — still needed.
            assert!(!evictable(i64::MAX, retention, &entry, false, true, false));
            // Protected: kept regardless of everything else.
            assert!(!evictable(i64::MAX, retention, &entry, true, true, true));
        }

        // AfterWatch: gone at the next pass.
        //   a watched playlist entry ...
        assert!(evictable(
            1_000,
            CacheRetention::AfterWatch,
            &entry,
            true,
            true,
            false
        ));
        //   ... and an unwatched file that has left the playlist.
        assert!(evictable(
            1_000,
            CacheRetention::AfterWatch,
            &entry,
            false,
            false,
            false
        ));

        // Keep(week): the retention window is driven by last_access and
        // applies to both disposable cases (watched, or not-in-playlist).
        for (watched, in_playlist) in [(true, true), (false, false)] {
            assert!(!evictable(
                1_000 + week_millis - 1,
                CacheRetention::Keep(week),
                &entry,
                watched,
                in_playlist,
                false
            ));
            assert!(evictable(
                1_000 + week_millis,
                CacheRetention::Keep(week),
                &entry,
                watched,
                in_playlist,
                false
            ));
        }

        // Infinite: never, disposable or not.
        assert!(!evictable(
            i64::MAX,
            CacheRetention::Infinite,
            &entry,
            true,
            true,
            false
        ));
        assert!(!evictable(
            i64::MAX,
            CacheRetention::Infinite,
            &entry,
            false,
            false,
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
        spawn_rig_clocked(storage, roots, retention, cache_dir, test_clock())
    }

    /// As [`spawn_rig_at`], with a caller-supplied clock — for tests that
    /// need "now" positioned relative to a file's real mtime (the
    /// startup orphan sweep).
    fn spawn_rig_clocked(
        storage: Storage,
        roots: Vec<PathBuf>,
        retention: CacheRetention,
        cache_dir: PathBuf,
        clock: Clock,
    ) -> Rig {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (out_tx, out_rx) = mpsc::channel(64);
        tokio::spawn(run(
            FileConfig {
                storage,
                media_roots: roots,
                retention,
                cache_dir,
                clock,
                download: DownloadConfig::default(),
                upload_limit: None,
                // No timer-driven scan in tests; drive via RescanLibrary.
                scan_interval: None,
                // Short deferral window so recheck/deferral tests run fast.
                scan_transfer_quiet: Duration::from_secs(2),
                torrent: None,
                nyaa: None,
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
        let (stream_tx, _stream_rx) = mpsc::channel(64);
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
                torrent: None,
                nyaa: None,
            },
            out_tx,
            done_tx,
            stream_tx,
        )
        .unwrap();

        let n = 250;
        let mut rebuilds = 0usize;
        let mut last_ptr = Arc::as_ptr(&actor.hash_cache);
        for i in 0..n {
            let path = PathBuf::from(format!("/media/ep{i}.mkv"));
            let hash = ed2k_hash_bytes(format!("episode {i}").as_bytes());
            actor.queue_scan_hash_commit(path, 1, hash, Path::new("/media"));
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

    /// The answered-request contract across a control-connection death:
    /// the transfer link task is aborted with stream opens still queued
    /// inside it, so their answers are lost with it.
    /// `TransferLinkReset` (bridged from the network's Disconnected
    /// event) must fail every pending queue so the next chunk-control
    /// send re-asks for a stream — pre-fix the queue-emptiness latch
    /// held "already asked" forever and the transfer stayed wedged
    /// across the reconnect.
    #[tokio::test]
    async fn transfer_link_reset_fails_pending_opens_so_the_next_send_re_asks() {
        let cache_dir = tempfile::tempdir().unwrap();
        let (out_tx, mut out_rx) = mpsc::channel(64);
        let (done_tx, _done_rx) = mpsc::channel(64);
        let (stream_tx, _stream_rx) = mpsc::channel(64);
        let mut actor = Actor::new(
            FileConfig {
                storage: Storage::open_in_memory().unwrap(),
                media_roots: vec![],
                retention: CacheRetention::default(),
                cache_dir: cache_dir.path().to_path_buf(),
                clock: test_clock(),
                download: DownloadConfig::default(),
                upload_limit: None,
                scan_interval: None,
                scan_transfer_quiet: Duration::from_secs(2),
                torrent: None,
                nyaa: None,
            },
            out_tx,
            done_tx,
            stream_tx,
        )
        .unwrap();

        let file = hash(1);
        let seed = PeerId::new("seed");
        actor
            .start_peer_download(file, 1024, vec![seed.clone()], 0)
            .await;
        while out_rx.try_recv().is_ok() {} // drain solicitation traffic

        let request = |chunks: Vec<u32>| PeerMessage::ChunkRequest { file, chunks };
        // The first chunk-control send queues and asks for a stream;
        // a second send while pending must not re-ask (the latch).
        actor
            .send_on_stream(seed.clone(), file, request(vec![0]))
            .await;
        assert!(
            matches!(out_rx.try_recv(), Ok(FileOutput::OpenTransfer { .. })),
            "the first queued message asks for a stream"
        );
        actor
            .send_on_stream(seed.clone(), file, request(vec![1]))
            .await;
        assert!(
            out_rx.try_recv().is_err(),
            "a queued key must not re-ask while its open is outstanding"
        );

        // The control connection dies: the open's answer is lost with
        // the aborted link task, and the reset stands in for it.
        actor.on_transfer_link_reset();
        assert!(
            actor.pending_streams.is_empty(),
            "the reset must fail every pending open"
        );

        // The next planned request re-asks for a stream.
        actor.send_on_stream(seed, file, request(vec![0])).await;
        assert!(
            matches!(out_rx.try_recv(), Ok(FileOutput::OpenTransfer { .. })),
            "a send after the reset must ask for a fresh stream"
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

    /// Regression: a removable filesystem may leave its mountpoint present
    /// while every file below it disappears.  Treating each missing path as
    /// an independent deletion throws away the complete hash index and makes
    /// reconnecting the filesystem re-hash the whole library.
    #[tokio::test]
    async fn rescan_retains_index_when_the_whole_root_disappears() {
        let root = tempfile::tempdir().unwrap();
        let first = write(root.path(), "Series/ep1.mkv", b"episode one");
        let second = write(root.path(), "Series/ep2.mkv", b"episode two");

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        for (path, contents) in [
            (&first, b"episode one".as_slice()),
            (&second, b"episode two".as_slice()),
        ] {
            let metadata = std::fs::metadata(path).unwrap();
            storage
                .upsert_hash_cache(
                    path,
                    mtime_millis(&metadata).unwrap(),
                    &ed2k_hash_bytes(contents),
                    1,
                )
                .unwrap();
        }
        std::fs::remove_file(&first).unwrap();
        std::fs::remove_file(&second).unwrap();

        let rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if Storage::open(&db_path)
                    .unwrap()
                    .library_roots()
                    .unwrap()
                    .iter()
                    .any(|state| state.vanished_at.is_some())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("scan never reconciled the missing paths");

        let check = Storage::open(&db_path).unwrap();
        assert_eq!(
            check.hash_cache().unwrap().len(),
            2,
            "a wholesale root disappearance must retain its cached hashes"
        );
        assert!(
            check.library_paths().unwrap().is_empty(),
            "vanished records must stay out of the active library"
        );
    }

    #[tokio::test]
    async fn reconnecting_vanished_root_reuses_cached_hashes() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("mount");
        std::fs::create_dir_all(&root).unwrap();
        let first = write(&root, "Series/ep1.mkv", b"episode one");
        let second = write(&root, "Series/ep2.mkv", b"episode two");

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let storage = Storage::open(&db_path).unwrap();
        for (path, contents) in [
            (&first, b"episode one".as_slice()),
            (&second, b"episode two".as_slice()),
        ] {
            let metadata = std::fs::metadata(path).unwrap();
            storage
                .upsert_hash_cache(
                    path,
                    mtime_millis(&metadata).unwrap(),
                    &ed2k_hash_bytes(contents),
                    1,
                )
                .unwrap();
        }

        // Model an unmounted dataset: its old tree is parked elsewhere and
        // the mountpoint itself remains as an empty directory.
        let parked = base.path().join("parked");
        std::fs::rename(&root, &parked).unwrap();
        std::fs::create_dir(&root).unwrap();
        let rig = spawn_rig(storage, vec![root.clone()], CacheRetention::default());
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if Storage::open(&db_path)
                    .unwrap()
                    .library_roots()
                    .unwrap()
                    .iter()
                    .any(|state| state.vanished_at.is_some())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        std::fs::remove_dir(&root).unwrap();
        std::fs::rename(&parked, &root).unwrap();
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();
        let mut outputs = rig.outputs;
        match tokio::time::timeout(Duration::from_secs(10), outputs.recv())
            .await
            .unwrap()
            .unwrap()
        {
            FileOutput::LibraryIndexed { files } => assert_eq!(files.len(), 2),
            other => panic!("expected cache-hit library result, got {other:?}"),
        }
        let check = Storage::open(&db_path).unwrap();
        assert_eq!(check.library_paths().unwrap().len(), 2);
        assert_eq!(check.library_roots().unwrap()[0].vanished_at, None);
    }

    #[tokio::test]
    async fn vanished_root_retracts_previously_ready_availability() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        let path = write(root.path(), "ep1.mkv", contents);
        let file = ed2k_hash_bytes(contents).root;
        let storage = Storage::open_in_memory().unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );
        rig.commands
            .send(FileCommand::Resolve {
                file,
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

        std::fs::remove_file(path).unwrap();
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();
        match next_output(&mut rig).await {
            FileOutput::Availability {
                file: got,
                availability,
            } => {
                assert_eq!(got, file);
                assert_eq!(availability, FileAvailability::Missing);
            }
            other => panic!("expected Missing availability, got {other:?}"),
        }
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
    /// propagating a mismatched encode to the group. And the refusal must be
    /// *spoken*, not silent (2026-07-05 review): a definitive mismatch
    /// replies `CannotServe`, so the requester stops soliciting a holder
    /// that can never answer instead of re-asking on every cooldown.
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

        // A peer solicits the requested (mismatched) hash. We must not serve
        // the hashes — we must answer CannotServe so it stops asking.
        rig.commands
            .send(FileCommand::PeerMessage {
                from: PeerId::new("peer7"),
                message: Box::new(PeerMessage::BlockHashRequest { file: requested }),
            })
            .await
            .unwrap();
        match tokio::time::timeout(Duration::from_millis(1000), next_output(&mut rig)).await {
            Ok(FileOutput::SendPeer { to, message }) => {
                assert_eq!(to, PeerId::new("peer7"));
                match *message {
                    PeerMessage::CannotServe { file } => assert_eq!(file, requested),
                    other => {
                        panic!("expected CannotServe for mismatched content, served: {other:?}")
                    }
                }
            }
            Ok(other) => panic!("unexpected output: {other:?}"),
            Err(_) => panic!("mismatched solicitation went unanswered (silent bail)"),
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
                subdirectory: true,
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
                subdirectory: true,
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Archived { result, .. } => assert!(result.is_err()),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn archive_can_move_directly_into_the_download_root() {
        let cache = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let contents = b"an unsorted cached episode".as_slice();
        let cached_path = write(cache.path(), "ep2.mkv", contents);
        let hashed = ed2k_hash_bytes(contents);

        let storage = Storage::open_in_memory().unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hashed.root,
                path: cached_path,
                size_bytes: contents.len() as u64,
                last_access: 1,
            })
            .unwrap();

        let expected = library.path().join("ep2.mkv");
        let mut rig = spawn_rig(
            storage,
            vec![library.path().to_path_buf()],
            CacheRetention::default(),
        );
        rig.commands
            .send(FileCommand::Archive {
                file: hashed.root,
                series_name: Some("Frieren".into()),
                filename: "ep2.mkv".into(),
                subdirectory: false,
            })
            .await
            .unwrap();

        match next_output(&mut rig).await {
            FileOutput::Archived { result, .. } => assert_eq!(result.unwrap(), expected),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(expected.is_file());
        assert!(!library.path().join("Frieren/ep2.mkv").exists());
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
                subdirectory: true,
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
                // All three are still playlist entries, so the unwatched
                // one is kept for being needed, not swept as unreferenced.
                playlist: HashSet::from([hash(1), hash(2), hash(3)]),
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
                playlist: HashSet::from([hashed.root]),
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
                // Still in the playlist: eviction is driven by the group
                // watched flag, not by being unreferenced.
                playlist: HashSet::from([hash(1)]),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Evicted { files } => assert_eq!(files, vec![hash(1)]),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(!behind_path.exists());
    }

    /// Regression: a file that has left the playlist entirely is evictable
    /// even though it was never watched — an abandoned download must not
    /// pin cache space forever. (design.md, Download Cache: a cached file
    /// is disposable once watched *or* no longer referenced.)
    #[tokio::test]
    async fn eviction_reclaims_files_no_longer_in_the_playlist() {
        let cache = tempfile::tempdir().unwrap();
        let gone = write(cache.path(), "gone.mkv", b"left the playlist");
        let storage = Storage::open_in_memory().unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: hash(1),
                path: gone.clone(),
                size_bytes: std::fs::metadata(&gone).unwrap().len(),
                last_access: 0,
            })
            .unwrap();
        // AfterWatch retention, never watched, and — crucially — the empty
        // playlist means it is unreferenced.
        let mut rig = spawn_rig(storage, vec![], CacheRetention::AfterWatch);
        rig.commands
            .send(FileCommand::RunEviction {
                protected: HashSet::new(),
                group_watched: HashSet::new(),
                playlist: HashSet::new(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Evicted { files } => assert_eq!(files, vec![hash(1)]),
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(!gone.exists());
    }

    /// Regression: a hash-named cache file with no `cache_entries` row is
    /// an orphan — a DB reset dropped the bookkeeping (files stay), or an
    /// abandoned peer-download partial (`download_path` is the final cache
    /// path). run_eviction only iterates cache_entries, so orphans are
    /// invisible to it and leak forever. Startup sweeps orphans older than
    /// a week; a recent one (possibly still in flight), a file that still
    /// has a row, and non-hash-named files are all left alone.
    #[tokio::test]
    async fn startup_sweeps_stale_orphaned_cache_files() {
        let cache = tempfile::tempdir().unwrap();

        // A stale orphan: hash-named, no DB row.
        let stale = ed2k_hash_bytes(b"stale orphan payload").root;
        let stale_path = cache.path().join(stale.to_string());
        std::fs::write(&stale_path, b"stale orphan payload").unwrap();
        let stale_mtime = mtime_millis(&std::fs::metadata(&stale_path).unwrap()).unwrap();

        // A second orphan: also hash-named and row-less.
        let orphan2 = ed2k_hash_bytes(b"second orphan payload").root;
        let orphan2_path = cache.path().join(orphan2.to_string());
        std::fs::write(&orphan2_path, b"second orphan payload").unwrap();

        // A tracked file (has a row): reconciled, never swept.
        let tracked = ed2k_hash_bytes(b"tracked payload").root;
        let tracked_path = cache.path().join(tracked.to_string());
        std::fs::write(&tracked_path, b"tracked payload").unwrap();

        // A non-hash-named file (e.g. the placeholder): ignored.
        let placeholder = cache.path().join("placeholder.png");
        std::fs::write(&placeholder, b"png").unwrap();

        let storage = Storage::open_in_memory().unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: tracked,
                path: tracked_path.clone(),
                size_bytes: std::fs::metadata(&tracked_path).unwrap().len(),
                last_access: 0,
            })
            .unwrap();

        // Clock a fortnight past the orphans' mtime, so anything row-less
        // and hash-named is past the week cutoff. (The `recent`/`stale`
        // naming just reflects intent; both freshly-written files share a
        // wall-clock mtime, so both are swept here. The under-the-cutoff
        // "keep a recent orphan" branch is covered by the next test, which
        // uses a clock only an hour past mtime.)
        let week = 7 * 24 * 3600 * 1000;
        let clock_old: Clock = Arc::new(move || (stale_mtime + 2 * week) as u64);
        let mut rig = spawn_rig_clocked(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().into(),
            clock_old,
        );
        // Synchronize: a Resolve reply guarantees Actor::new (and its
        // synchronous sweep) has completed.
        rig.commands
            .send(FileCommand::Resolve {
                file: tracked,
                filename: "tracked.mkv".into(),
            })
            .await
            .unwrap();
        let _ = next_output(&mut rig).await;

        assert!(!stale_path.exists(), "stale orphan must be swept");
        assert!(
            !orphan2_path.exists(),
            "the second stale orphan is swept too"
        );
        assert!(tracked_path.exists(), "a file with a cache row is kept");
        assert!(placeholder.exists(), "non-hash-named files are ignored");
    }

    /// The age cutoff: an orphan younger than a week is left alone (it may
    /// be an in-flight or just-abandoned partial we shouldn't race).
    #[tokio::test]
    async fn startup_keeps_recent_orphaned_cache_files() {
        let cache = tempfile::tempdir().unwrap();
        let recent = ed2k_hash_bytes(b"recent partial").root;
        let recent_path = cache.path().join(recent.to_string());
        std::fs::write(&recent_path, b"recent partial").unwrap();
        let mtime = mtime_millis(&std::fs::metadata(&recent_path).unwrap()).unwrap();

        let storage = Storage::open_in_memory().unwrap();
        // Clock only an hour past the file's mtime: under the week cutoff.
        let clock: Clock = Arc::new(move || (mtime + 3600 * 1000) as u64);
        let mut rig = spawn_rig_clocked(
            storage,
            vec![],
            CacheRetention::default(),
            cache.path().into(),
            clock,
        );
        // Resolve a missing file to synchronize past Actor::new.
        rig.commands
            .send(FileCommand::Resolve {
                file: ed2k_hash_bytes(b"nothing").root,
                filename: "nothing.mkv".into(),
            })
            .await
            .unwrap();
        let _ = next_output(&mut rig).await;

        assert!(recent_path.exists(), "a recent orphan must be left alone");
    }

    // ---- media-library scan.

    /// Build an in-memory hash-cache map keyed by `path`, trusting the
    /// file's current mtime (so the entry counts as a cache hit).
    fn cache_of(path: &Path, contents: &[u8]) -> HashMap<PathBuf, (i64, Ed2kFileHash)> {
        let mtime = mtime_millis(&std::fs::metadata(path).unwrap()).unwrap();
        HashMap::from([(path.to_path_buf(), (mtime, ed2k_hash_bytes(contents)))])
    }

    proptest! {
        #[test]
        fn root_disappearance_policy_depends_only_on_whether_any_record_survives(
            exists in proptest::collection::vec(any::<bool>(), 0..128)
        ) {
            let expected = if exists.is_empty() {
                RootDisposition::Empty
            } else if exists.iter().any(|value| *value) {
                RootDisposition::Online
            } else {
                RootDisposition::Vanished
            };
            prop_assert_eq!(root_disposition(&exists), expected);
        }
    }

    #[test]
    fn scan_library_classifies_hits_worklist_and_skips_non_video() {
        let root = tempfile::tempdir().unwrap();
        let video = write(root.path(), "Frieren/ep1.mkv", b"episode one");
        // Junk files must never be hashed or indexed.
        write(root.path(), "Frieren/poster.jpg", b"jpeg");
        write(root.path(), "Frieren/ep1.nfo", b"<nfo/>");

        // Empty cache: the video needs hashing, the junk is ignored.
        let (hits, worklist, _stale, _, _) =
            scan_library(&[root.path().to_path_buf()], &HashMap::new());
        assert!(hits.is_empty());
        assert_eq!(worklist.len(), 1);
        assert_eq!(worklist[0].path, video);
        assert_eq!(worklist[0].filename, "ep1.mkv");

        // With the video already cached (matching mtime/size) it's a hit,
        // not re-hashed.
        let cache = cache_of(&video, b"episode one");
        let (hits, worklist, _stale, _, _) = scan_library(&[root.path().to_path_buf()], &cache);
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
        let (hits, worklist, _stale, _, _) = scan_library(&[root.path().to_path_buf()], &cache);
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

        let (hits, worklist, _stale, _, _) =
            scan_library(std::slice::from_ref(&root), &HashMap::new());
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

        let (hits, worklist, _stale, _, _) =
            scan_library(std::slice::from_ref(&root), &HashMap::new());
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

        let (_hits, worklist, _stale, _, _) =
            scan_library(&[root.path().to_path_buf()], &HashMap::new());
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

    /// Regression (2026-07-03): while a playlist entry was downloading
    /// from a peer, the same file landed in a media root through another
    /// channel (a bittorrent download). The library walk saw it, but scan
    /// hashing defers during transfers (#21) — and the download itself is
    /// transfer traffic — so the local copy was never discovered until a
    /// restart. A file appearing under a media root bearing the name of
    /// an entry we're downloading must be verified promptly (outside the
    /// scan deferral), the peer download cancelled and its partial file
    /// removed, and the entry resolved Verified at the media-root path.
    #[tokio::test]
    async fn a_local_copy_appearing_mid_download_cancels_it_and_resolves() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"the episode the group is watching".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        let file = hashed.root;

        let db = tempfile::tempdir().unwrap();
        let storage = Storage::open(&db.path().join("test.db")).unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );

        // The entry has no local copy yet.
        rig.commands
            .send(FileCommand::Resolve {
                file,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { resolution, .. } => {
                assert!(matches!(resolution, Resolution::NotFound));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        // A peer download starts — transfer traffic from here on.
        let peer = dessplay_core::net::PeerId::from(dessplay_core::types::UserId::new("kim"));
        rig.commands
            .send(FileCommand::StartDownload {
                file,
                size_bytes: contents.len() as u64,
                sources: vec![peer],
                play_chunk: 0,
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::SendPeer { message, .. } => {
                assert!(matches!(
                    *message,
                    dessplay_core::net::PeerMessage::BlockHashRequest { .. }
                ));
            }
            other => panic!("unexpected output: {other:?}"),
        }
        let partial = rig
            ._cache_dir
            .as_ref()
            .unwrap()
            .path()
            .join(file.to_string());
        assert!(partial.exists(), "download must have opened its partial");

        // The same file lands in a media root behind our back.
        write(root.path(), "Anime/ep1.mkv", contents);
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();

        // The actor must discover, verify, and adopt the local copy.
        let path = loop {
            match next_output(&mut rig).await {
                FileOutput::Resolved {
                    file: f,
                    resolution: Resolution::Verified(path),
                } if f == file => break path,
                _ => continue,
            }
        };
        assert_eq!(path, root.path().join("Anime/ep1.mkv"));
        // The peer download is cancelled and its partial cleaned up.
        assert!(
            !partial.exists(),
            "cancelled download must remove its partial file"
        );
    }

    /// The by-hash half of the same recovery: a copy of a missing entry
    /// that lands under a *different filename* is invisible to the
    /// name-based walk trigger and to `resolve`, but the scan hashes it
    /// eventually — matching contents must still be adopted (resolved
    /// Verified at the scanned path).
    #[tokio::test]
    async fn a_renamed_local_copy_is_adopted_by_hash_when_scanned() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"the same episode under another name".as_slice();
        let file = ed2k_hash_bytes(contents).root;

        let db = tempfile::tempdir().unwrap();
        let storage = Storage::open(&db.path().join("test.db")).unwrap();
        let mut rig = spawn_rig(
            storage,
            vec![root.path().to_path_buf()],
            CacheRetention::default(),
        );

        // The entry is missing under its playlist name.
        rig.commands
            .send(FileCommand::Resolve {
                file,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::Resolved { resolution, .. } => {
                assert!(matches!(resolution, Resolution::NotFound));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        // The content arrives under a different name; the scan hashes it.
        let renamed = write(root.path(), "renamed.mkv", contents);
        rig.commands.send(FileCommand::RescanLibrary).await.unwrap();

        let path = loop {
            match next_output(&mut rig).await {
                FileOutput::Resolved {
                    file: f,
                    resolution: Resolution::Verified(path),
                } if f == file => break path,
                _ => continue,
            }
        };
        assert_eq!(path, renamed);
    }

    /// Regression (2026-08-12 review): the by-hash cache candidate is
    /// the live download's own partial. Offering it while the download
    /// is active costs a full ed2k pass over a moving, full-size sparse
    /// file on every resolve (its mtime churn defeats the hash cache),
    /// and a partial must never resolve Verified out from under the
    /// scheduler. While `downloads.is_active`, resolve must skip the
    /// `<cache>/<hash>` candidate entirely.
    #[tokio::test]
    async fn resolve_skips_the_cache_candidate_while_its_download_is_active() {
        let contents = b"a partial that happens to look complete".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        let file = hashed.root;

        let cache = tempfile::tempdir().unwrap();
        let mut rig = spawn_rig_at(
            Storage::open_in_memory().unwrap(),
            vec![],
            CacheRetention::default(),
            cache.path().to_path_buf(),
        );

        // An active download for the file (one silent source).
        rig.commands
            .send(FileCommand::StartDownload {
                file,
                size_bytes: contents.len() as u64,
                sources: vec![PeerId::new("src")],
                play_chunk: 0,
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::SendPeer { message, .. } => {
                assert!(matches!(*message, PeerMessage::BlockHashRequest { .. }));
            }
            other => panic!("unexpected output: {other:?}"),
        }

        // The partial at the cache path holds verifying bytes (the moving
        // file just happens to look complete at this instant).
        std::fs::write(cache.path().join(file.to_string()), contents).unwrap();

        rig.commands
            .send(FileCommand::Resolve {
                file,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        loop {
            match next_output(&mut rig).await {
                FileOutput::Resolved {
                    file: f,
                    resolution,
                } if f == file => {
                    assert_eq!(
                        resolution,
                        Resolution::NotFound,
                        "the live partial must not be offered as a resolve candidate"
                    );
                    break;
                }
                _ => continue,
            }
        }
    }

    // ---- Nyaa browse-import tests (fake engine).

    use crate::torrent::engine::{FakeTorrentEngine, TorrentStatus};
    use crate::torrent::nyaa::NyaaMatch;

    /// A nyaa source for rigs whose tests never run a browse search
    /// (the import tests hand the actor a pre-inspected result).
    struct NoNyaa;

    impl NyaaSource for NoNyaa {
        fn search_anime(&self, _query: &str) -> std::io::Result<String> {
            Err(std::io::Error::other("no search in this test"))
        }

        fn fetch_torrent(&self, _url: &str) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::other("no metadata in this test"))
        }
    }

    fn spawn_torrent_rig(roots: Vec<PathBuf>, clock: Clock) -> (Rig, Arc<FakeTorrentEngine>) {
        spawn_torrent_rig_with_storage(Storage::open_in_memory().unwrap(), roots, clock)
    }

    /// As [`spawn_torrent_rig`], with caller-supplied storage (so a test
    /// can reopen the database and inspect cache bookkeeping).
    fn spawn_torrent_rig_with_storage(
        storage: Storage,
        roots: Vec<PathBuf>,
        clock: Clock,
    ) -> (Rig, Arc<FakeTorrentEngine>) {
        let cache_dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(FakeTorrentEngine::default());
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (out_tx, out_rx) = mpsc::channel(64);
        tokio::spawn(run(
            FileConfig {
                storage,
                media_roots: roots,
                retention: CacheRetention::AfterWatch,
                cache_dir: cache_dir.path().to_path_buf(),
                clock,
                download: DownloadConfig::default(),
                upload_limit: None,
                scan_interval: None,
                scan_transfer_quiet: Duration::from_secs(2),
                torrent: Some(engine.clone() as Arc<dyn TorrentEngine>),
                nyaa: Some(Arc::new(NoNyaa)),
            },
            cmd_rx,
            out_tx,
        ));
        (
            Rig {
                commands: cmd_tx,
                outputs: out_rx,
                _cache_dir: Some(cache_dir),
            },
            engine,
        )
    }

    /// Wait until the fake engine has been handed the import.
    async fn wait_import_added(engine: &FakeTorrentEngine, id: TorrentImportId) -> PathBuf {
        for _ in 0..100 {
            if let Some((_, dir)) = engine.added_import(id) {
                return dir;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("import was never added to the engine");
    }

    /// Drive a browse import to a verified, promoted completion: start
    /// it, materialize the payload, script the engine finished, and wait
    /// for the successful finish. Returns the imported file's hash.
    async fn complete_import(
        rig: &mut Rig,
        engine: &FakeTorrentEngine,
        id: TorrentImportId,
        contents: &[u8],
    ) -> Ed2kHash {
        complete_import_result(rig, engine, id, contents)
            .await
            .0
            .root
    }

    /// As [`complete_import`], returning the full finish payload (the
    /// hash and the local path the import finished against).
    async fn complete_import_result(
        rig: &mut Rig,
        engine: &FakeTorrentEngine,
        id: TorrentImportId,
        contents: &[u8],
    ) -> (Ed2kFileHash, PathBuf) {
        rig.commands
            .send(FileCommand::StartNyaaImport {
                id,
                result: browse_result("chosen.mkv", contents.len() as u64),
                after: None,
            })
            .await
            .unwrap();
        let out_dir = wait_import_added(engine, id).await;
        std::fs::create_dir_all(&out_dir).unwrap();
        let payload = out_dir.join("chosen.mkv");
        std::fs::write(&payload, contents).unwrap();
        engine.set_import_status(
            id,
            TorrentStatus {
                progress_bytes: contents.len() as u64,
                finished: true,
                error: false,
                payload: Some(payload),
            },
        );
        loop {
            if let FileOutput::NyaaImportFinished { result, .. } = next_output(rig).await {
                return result.expect("import must finish cleanly");
            }
        }
    }

    fn browse_result(filename: &str, size_bytes: u64) -> NyaaBrowseResult {
        NyaaBrowseResult {
            title: format!("Release {filename}"),
            filename: filename.to_string(),
            size_bytes,
            seeders: 10,
            chosen: NyaaMatch {
                title: format!("Release {filename}"),
                torrent_url: "https://nyaa.si/download/99.torrent".into(),
                info_hash: "0123456789abcdef0123456789abcdef01234567".into(),
            },
        }
    }

    #[tokio::test]
    async fn selected_nyaa_import_hashes_promotes_and_finishes() {
        let contents = b"selected current-season episode".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        let id = TorrentImportId(7);
        let (mut rig, engine) = spawn_torrent_rig(vec![], test_clock());
        rig.commands
            .send(FileCommand::StartNyaaImport {
                id,
                result: browse_result("chosen.mkv", contents.len() as u64),
                after: Some(hash(9)),
            })
            .await
            .unwrap();
        assert!(matches!(
            next_output(&mut rig).await,
            FileOutput::NyaaImportProgress {
                id: TorrentImportId(7),
                stage: NyaaImportStage::Downloading,
                ..
            }
        ));
        let (_, out_dir) = engine.added_import(id).expect("import added");
        std::fs::create_dir_all(&out_dir).unwrap();
        let payload = out_dir.join("chosen.mkv");
        std::fs::write(&payload, contents).unwrap();
        engine.set_import_status(
            id,
            TorrentStatus {
                progress_bytes: contents.len() as u64,
                finished: true,
                error: false,
                payload: Some(payload),
            },
        );
        let mut saw_hashing = false;
        let finished = loop {
            match next_output(&mut rig).await {
                FileOutput::NyaaImportProgress {
                    stage: NyaaImportStage::Hashing,
                    ..
                } => saw_hashing = true,
                FileOutput::NyaaImportFinished { result, after, .. } => {
                    assert_eq!(after, Some(hash(9)));
                    break result;
                }
                _ => {}
            }
        };
        let (actual, path) = finished.unwrap();
        assert!(saw_hashing);
        assert_eq!(actual.root, hashed.root);
        assert!(path.exists());
        assert!(
            engine.added(&hashed.root).is_some(),
            "import must be promoted"
        );
        assert!(engine.added_import(id).is_none());
    }

    /// Regression (2026-08-12 review; regressed from 2026-07-19): a
    /// completed browse import never cancelled the in-flight peer
    /// download of the same hash. The orphaned download kept running:
    /// its next progress overwrote Ready with `Downloading` (gating the
    /// group on a user holding a complete verified copy),
    /// `place_in_cache` unlinked `<cache>/<hash>` under the live
    /// ChunkStore fd, and late chunks "completed" the download a second
    /// time.
    #[tokio::test]
    async fn nyaa_import_completion_cancels_the_redundant_peer_download() {
        let chunk = dessplay_core::net::CHUNK_SIZE as usize;
        let contents: Vec<u8> = (0..chunk + 16).map(|i| (i % 251) as u8).collect();
        let hashed = ed2k_hash_bytes(&contents);
        let file = hashed.root;
        let peer = PeerId::new("seeder");
        let (mut rig, engine) = spawn_torrent_rig(vec![], test_clock());

        // An active peer download for the same file, far enough along
        // to have chunk requests in flight.
        rig.commands
            .send(FileCommand::StartDownload {
                file,
                size_bytes: contents.len() as u64,
                sources: vec![peer.clone()],
                play_chunk: 0,
            })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::SendPeer { message, .. } => {
                assert!(matches!(*message, PeerMessage::BlockHashRequest { .. }));
            }
            other => panic!("unexpected output: {other:?}"),
        }
        rig.commands
            .send(FileCommand::PeerMessage {
                from: peer.clone(),
                message: Box::new(PeerMessage::BlockHashes {
                    file,
                    hashes: hashed.blocks.clone(),
                }),
            })
            .await
            .unwrap();
        let chunks = dessplay_core::net::chunk_count(contents.len() as u64);
        let mut bitfield = dessplay_core::net::Bitfield::new(chunks);
        for index in 0..chunks {
            bitfield.set(index);
        }
        rig.commands
            .send(FileCommand::PeerMessage {
                from: peer.clone(),
                message: Box::new(PeerMessage::FileAvailability { file, bitfield }),
            })
            .await
            .unwrap();
        // The chunk requests head for a data stream; the open request
        // proves they are in flight.
        loop {
            match next_output(&mut rig).await {
                FileOutput::OpenTransfer { file: f, .. } if f == file => break,
                _ => continue,
            }
        }

        // The browse import of the same content completes.
        let (_, local_path) =
            complete_import_result(&mut rig, &engine, TorrentImportId(21), &contents).await;
        let cache_path = rig
            ._cache_dir
            .as_ref()
            .unwrap()
            .path()
            .join(file.to_string());
        assert_eq!(local_path, cache_path);

        // The source's late chunks arrive. The cancelled download must
        // ignore them; before the fix they completed the orphaned
        // download a second time.
        for index in 0..chunks {
            let start = index as usize * chunk;
            let end = (start + chunk).min(contents.len());
            rig.commands
                .send(FileCommand::PeerMessage {
                    from: peer.clone(),
                    message: Box::new(PeerMessage::ChunkData {
                        file,
                        index,
                        data: contents[start..end].to_vec(),
                    }),
                })
                .await
                .unwrap();
        }
        // Sentinel: an unrelated resolve flushes the pipeline.
        rig.commands
            .send(FileCommand::Resolve {
                file: hash(42),
                filename: "zz.mkv".into(),
            })
            .await
            .unwrap();
        loop {
            match next_output(&mut rig).await {
                FileOutput::Availability {
                    file: f,
                    availability,
                } if f == file => {
                    panic!("no availability write may follow the import's Ready: {availability:?}");
                }
                FileOutput::DownloadComplete { file: f, .. } if f == file => {
                    panic!("the orphaned peer download completed a second time");
                }
                FileOutput::Resolved { file: f, .. } if f == hash(42) => break,
                _ => continue,
            }
        }
        // The cached copy survived: the cancel deleted the partial
        // *before* the payload was placed at the same path.
        assert_eq!(std::fs::read(&cache_path).unwrap(), contents);
    }

    /// Still-open from 2026-07-19 (re-verified 2026-08-12): a browse
    /// import of a byte-identical file already under a media root
    /// overwrote `local_files` with the cache path and added a
    /// `cache_entries` row — retention would later evict the "cached"
    /// copy and flip the client Missing for a file that never left its
    /// library.
    #[tokio::test]
    async fn nyaa_import_of_an_already_held_file_keeps_the_library_copy() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"an episode the library already holds".as_slice();
        let hashed = ed2k_hash_bytes(contents);
        let library_path = write(root.path(), "Anime/ep1.mkv", contents);

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("test.db");
        let (mut rig, engine) = spawn_torrent_rig_with_storage(
            Storage::open(&db_path).unwrap(),
            vec![root.path().to_path_buf()],
            test_clock(),
        );

        // The library copy resolves first, so the actor holds it.
        rig.commands
            .send(FileCommand::Resolve {
                file: hashed.root,
                filename: "ep1.mkv".into(),
            })
            .await
            .unwrap();
        loop {
            match next_output(&mut rig).await {
                FileOutput::Resolved { resolution, .. } => {
                    assert_eq!(resolution, Resolution::Verified(library_path.clone()));
                    break;
                }
                _ => continue,
            }
        }

        let (_, local_path) =
            complete_import_result(&mut rig, &engine, TorrentImportId(22), contents).await;
        assert_eq!(
            local_path, library_path,
            "the import must finish against the library copy"
        );

        // No cache copy shadows the library file...
        let cache_path = rig
            ._cache_dir
            .as_ref()
            .unwrap()
            .path()
            .join(hashed.root.to_string());
        assert!(
            !cache_path.exists(),
            "no cache copy may be created for a file already in the library"
        );
        // ...and no cache_entries row was written for it (a row would
        // make retention evict a copy the library still holds).
        let check = Storage::open(&db_path).unwrap();
        assert!(
            check.cache_entries().unwrap().is_empty(),
            "no cache_entries row may shadow the library copy"
        );
        // The torrent still seeds (promoted), so the live-disable path
        // can find it by hash.
        assert!(engine.added(&hashed.root).is_some());
    }

    #[tokio::test]
    async fn selected_nyaa_import_can_be_cancelled() {
        let id = TorrentImportId(8);
        let (mut rig, engine) = spawn_torrent_rig(vec![], test_clock());
        rig.commands
            .send(FileCommand::StartNyaaImport {
                id,
                result: browse_result("cancel.mkv", 100),
                after: None,
            })
            .await
            .unwrap();
        let _ = next_output(&mut rig).await;
        assert!(engine.added_import(id).is_some());
        rig.commands
            .send(FileCommand::CancelNyaaImport { id })
            .await
            .unwrap();
        match next_output(&mut rig).await {
            FileOutput::NyaaImportFinished { result, .. } => {
                assert_eq!(result.unwrap_err(), "Cancelled");
            }
            other => panic!("unexpected output: {other:?}"),
        }
        assert!(engine.added_import(id).is_none());
    }

    /// The live BitTorrent toggle (design.md, BitTorrent Downloads):
    /// disabling removes a completed import's still-seeding torrent
    /// (files deleted) — the mid-session escape hatch for a saturated
    /// uplink. The cached copy survives.
    #[tokio::test]
    async fn disable_removes_seeding_torrents() {
        let contents = b"an episode the uplink cannot afford".as_slice();
        let (mut rig, engine) = spawn_torrent_rig(vec![], test_clock());
        let file = complete_import(&mut rig, &engine, TorrentImportId(3), contents).await;
        assert!(engine.added(&file).is_some(), "import seeds after promote");

        rig.commands
            .send(FileCommand::SetTorrentEnabled(false))
            .await
            .unwrap();
        for _ in 0..100 {
            if engine.removed() == vec![(file, true)] {
                let cache_path = rig
                    ._cache_dir
                    .as_ref()
                    .unwrap()
                    .path()
                    .join(file.to_string());
                assert_eq!(
                    std::fs::read(&cache_path).unwrap(),
                    contents,
                    "the cached copy must survive the disable"
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("disable never removed the seeding torrent");
    }

    /// Disabling BitTorrent cancels pending user-selected imports (they
    /// cannot finish without the engine) instead of leaving them stuck.
    #[tokio::test]
    async fn disable_cancels_pending_nyaa_imports() {
        let id = TorrentImportId(11);
        let (mut rig, engine) = spawn_torrent_rig(vec![], test_clock());
        rig.commands
            .send(FileCommand::StartNyaaImport {
                id,
                result: browse_result("chosen.mkv", 1000),
                after: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            next_output(&mut rig).await,
            FileOutput::NyaaImportProgress { .. }
        ));
        rig.commands
            .send(FileCommand::SetTorrentEnabled(false))
            .await
            .unwrap();
        loop {
            if let FileOutput::NyaaImportFinished { result, .. } = next_output(&mut rig).await {
                assert_eq!(result.unwrap_err(), "BitTorrent was disabled.");
                break;
            }
        }
        assert!(engine.added_import(id).is_none(), "import removed");
    }

    /// While the setting is off, a browse import is refused with a
    /// pointer to Settings instead of silently starting the engine.
    #[tokio::test]
    async fn import_while_disabled_is_refused() {
        let (mut rig, engine) = spawn_torrent_rig(vec![], test_clock());
        rig.commands
            .send(FileCommand::SetTorrentEnabled(false))
            .await
            .unwrap();
        rig.commands
            .send(FileCommand::StartNyaaImport {
                id: TorrentImportId(4),
                result: browse_result("refused.mkv", 100),
                after: None,
            })
            .await
            .unwrap();
        loop {
            if let FileOutput::NyaaImportFinished { result, .. } = next_output(&mut rig).await {
                assert_eq!(
                    result.unwrap_err(),
                    "BitTorrent downloads are disabled; enable them in Settings."
                );
                break;
            }
        }
        assert!(engine.added_import(TorrentImportId(4)).is_none());
    }

    /// Eviction ends seeding: evicting the cached file also removes the
    /// promoted import's torrent (and its payload) from the engine.
    #[tokio::test]
    async fn eviction_removes_the_torrent() {
        let contents = b"a watched episode past retention".as_slice();
        let (mut rig, engine) = spawn_torrent_rig(vec![], test_clock());
        let file = complete_import(&mut rig, &engine, TorrentImportId(5), contents).await;

        // Watched by the group, AfterWatch retention: evict now.
        rig.commands
            .send(FileCommand::RunEviction {
                protected: HashSet::new(),
                group_watched: [file].into(),
                playlist: [file].into(),
            })
            .await
            .unwrap();
        loop {
            if let FileOutput::Evicted { files } = next_output(&mut rig).await {
                assert_eq!(files, vec![file]);
                break;
            }
        }
        assert_eq!(engine.removed(), vec![(file, true)]);
        assert!(
            !rig._cache_dir
                .as_ref()
                .unwrap()
                .path()
                .join(file.to_string())
                .exists(),
            "evicted cache file must be gone"
        );
    }

    /// Startup sweeps everything under `<cache>/torrents/` — torrents
    /// never survive a restart, so abandoned import dirs, prior
    /// versions' per-hash payload dirs, and a legacy librqbit session
    /// dir are all garbage. The one exception: a dir hosting a
    /// *registered cache file* (a completed import whose hardlink into
    /// the cache failed and was registered in place) must be spared.
    /// The sweep runs with the engine disabled too — that is exactly
    /// when leftovers would otherwise linger forever.
    #[tokio::test]
    async fn startup_sweeps_stale_torrents_dir() {
        let contents = b"a cache entry living inside its import dir".as_slice();
        let kept = ed2k_hash_bytes(contents).root;

        let storage = Storage::open_in_memory().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let torrents_dir = cache_dir.path().join("torrents");
        // A cache entry registered in place inside an import dir (the
        // failed-hardlink fallback).
        let kept_dir = torrents_dir.join("import-5");
        std::fs::create_dir_all(&kept_dir).unwrap();
        let kept_path = kept_dir.join("kept.mkv");
        std::fs::write(&kept_path, contents).unwrap();
        storage
            .upsert_cache_entry(&CacheEntry {
                hash: kept,
                path: kept_path.clone(),
                size_bytes: contents.len() as u64,
                last_access: 0,
            })
            .unwrap();
        // Garbage: an abandoned import, a prior version's per-hash
        // payload dir, and a legacy librqbit session dir.
        let abandoned = torrents_dir.join("import-1");
        let legacy_hash_dir = torrents_dir.join(ed2k_hash_bytes(b"legacy").root.to_string());
        let legacy_session = torrents_dir.join(".session");
        for dir in [&abandoned, &legacy_hash_dir, &legacy_session] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("leftover"), b"stale").unwrap();
        }

        let mut rig = spawn_rig_at(
            storage,
            vec![],
            CacheRetention::Infinite,
            cache_dir.path().to_path_buf(),
        );
        // The sweep runs before commands are processed; use a resolve
        // round-trip as the startup barrier.
        rig.commands
            .send(FileCommand::Resolve {
                file: hash(1),
                filename: "barrier.mkv".into(),
            })
            .await
            .unwrap();
        let _ = next_output(&mut rig).await;

        assert!(!abandoned.exists(), "abandoned import dir must be swept");
        assert!(
            !legacy_hash_dir.exists(),
            "legacy per-hash payload dir must be swept"
        );
        assert!(
            !legacy_session.exists(),
            "legacy librqbit session dir must be swept"
        );
        assert!(
            kept_path.exists(),
            "a dir hosting a registered cache file must be spared"
        );
    }
}
