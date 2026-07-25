//! The torrent-engine seam: what the file actor needs from a BitTorrent
//! implementation, small enough to fake in tests. The production
//! implementation (librqbit) lives in [`super::rqbit`]; construction
//! sites pass `None` to disable the torrent path entirely (tests, or a
//! build without the engine).
//!
//! The interface is deliberately poll-based: `add`/`remove` are
//! fire-and-forget, and the actor reads [`TorrentEngine::status`] on its
//! existing tick — no callback channel to plumb, and the fake is a plain
//! mutable map.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use dessplay_core::types::Ed2kHash;

use super::nyaa::NyaaMatch;

/// Local identity for a torrent selected before its ed2k hash is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TorrentImportId(pub u64);

/// A running (or finished, still-seeding) torrent's observable state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TorrentStatus {
    /// Payload bytes downloaded so far.
    pub progress_bytes: u64,
    /// The payload is completely downloaded (and seeding).
    pub finished: bool,
    /// The engine gave up on this torrent (tracker/metadata/IO error).
    pub error: bool,
    /// Where the payload file is, once metadata is known. For a
    /// multi-file torrent (unexpected for an exact-filename match) the
    /// engine reports the largest file.
    pub payload: Option<PathBuf>,
}

/// Aggregate live transfer speeds across every torrent in the engine,
/// for the status field's bandwidth display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TorrentSpeeds {
    /// Download, bytes/sec.
    pub down_bps: u64,
    /// Upload (seeding), bytes/sec.
    pub up_bps: u64,
}

/// What the file actor needs from a torrent implementation.
pub trait TorrentEngine: Send + Sync + 'static {
    /// Start downloading `chosen` into `output_dir` (created on demand).
    /// Idempotent: re-adding an already-known file is a no-op.
    fn add(&self, file: Ed2kHash, chosen: &NyaaMatch, output_dir: PathBuf);
    /// Remove the torrent for `file`, deleting its files when asked.
    /// A no-op for an unknown file.
    fn remove(&self, file: Ed2kHash, delete_files: bool);
    /// The torrent's current state, `None` if unknown to the engine.
    fn status(&self, file: &Ed2kHash) -> Option<TorrentStatus>;
    /// Every file the engine has a torrent for (startup reconciliation).
    fn active(&self) -> Vec<Ed2kHash>;
    /// Start a user-selected torrent whose ed2k identity is not known yet.
    fn add_import(&self, id: TorrentImportId, chosen: &NyaaMatch, output_dir: PathBuf);
    /// Remove a pending user-selected torrent.
    fn remove_import(&self, id: TorrentImportId, delete_files: bool);
    /// Poll a pending user-selected torrent.
    fn import_status(&self, id: TorrentImportId) -> Option<TorrentStatus>;
    /// Re-key a completed import so normal cache eviction and restart
    /// reconciliation own its seeding lifecycle.
    fn promote_import(&self, id: TorrentImportId, file: Ed2kHash);
    /// Aggregate live up/down speeds across every torrent, for the
    /// status field's bandwidth display. Default: no measurable
    /// traffic.
    fn speeds(&self) -> TorrentSpeeds {
        TorrentSpeeds::default()
    }
    /// Startup-only: reconcile the implementation's own persisted session
    /// against the app registry. `keep` maps a registered torrent's info
    /// hash (lowercase hex) to its file; a session-restored torrent whose
    /// info hash is not in `keep` is deleted along with its files —
    /// in-flight downloads (abandoned imports, mid-download leftovers)
    /// don't survive restarts (design.md, BitTorrent Downloads). Kept
    /// torrents are adopted under their file key so later `remove`/`status`
    /// calls see them. Returns the output directories kept torrents still
    /// use, so the caller can sweep abandoned per-import directories
    /// without racing a live one.
    fn reconcile_session(&self, keep: &HashMap<String, Ed2kHash>) -> Vec<PathBuf>;
}

/// In-memory fake for actor tests: `add`/`remove` record themselves,
/// and tests script `status` via [`FakeTorrentEngine::set_status`].
#[derive(Default)]
pub struct FakeTorrentEngine {
    inner: Mutex<FakeInner>,
}

#[derive(Default)]
struct FakeInner {
    torrents: HashMap<Ed2kHash, (NyaaMatch, PathBuf)>,
    status: HashMap<Ed2kHash, TorrentStatus>,
    removed: Vec<(Ed2kHash, bool)>,
    imports: HashMap<TorrentImportId, (NyaaMatch, PathBuf)>,
    import_status: HashMap<TorrentImportId, TorrentStatus>,
    removed_imports: Vec<(TorrentImportId, bool)>,
    /// Torrents the fake "session" restored from its own persistence
    /// (info hash → output dir), as librqbit's fastresume does; consumed
    /// by `reconcile_session`.
    restored: HashMap<String, PathBuf>,
    /// Info hashes `reconcile_session` deleted from the fake session.
    session_deleted: Vec<String>,
    /// Scripted aggregate speeds ([`TorrentEngine::speeds`]).
    speeds: TorrentSpeeds,
}

impl FakeTorrentEngine {
    /// Script the status the next `status` call reports for `file`.
    pub fn set_status(&self, file: Ed2kHash, status: TorrentStatus) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.status.insert(file, status);
        }
    }

    /// What was added for `file`, if anything.
    pub fn added(&self, file: &Ed2kHash) -> Option<(NyaaMatch, PathBuf)> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.torrents.get(file).cloned())
    }

    /// Every `remove` call seen, in order.
    pub fn removed(&self) -> Vec<(Ed2kHash, bool)> {
        self.inner
            .lock()
            .map(|inner| inner.removed.clone())
            .unwrap_or_default()
    }

    /// Script pending-import status.
    pub fn set_import_status(&self, id: TorrentImportId, status: TorrentStatus) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.import_status.insert(id, status);
        }
    }

    /// What was added for a pending import.
    pub fn added_import(&self, id: TorrentImportId) -> Option<(NyaaMatch, PathBuf)> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.imports.get(&id).cloned())
    }

    /// Model a torrent the underlying session restored from its own
    /// persistence at startup (as librqbit's fastresume does).
    pub fn restore_session_torrent(&self, info_hash: &str, output_dir: PathBuf) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.restored.insert(info_hash.to_string(), output_dir);
        }
    }

    /// Info hashes `reconcile_session` deleted from the fake session.
    pub fn session_deleted(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|inner| inner.session_deleted.clone())
            .unwrap_or_default()
    }

    /// Script the aggregate speeds [`TorrentEngine::speeds`] reports.
    pub fn set_speeds(&self, speeds: TorrentSpeeds) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.speeds = speeds;
        }
    }
}

impl TorrentEngine for FakeTorrentEngine {
    fn add(&self, file: Ed2kHash, chosen: &NyaaMatch, output_dir: PathBuf) {
        if let Ok(mut inner) = self.inner.lock() {
            inner
                .torrents
                .entry(file)
                .or_insert_with(|| (chosen.clone(), output_dir));
        }
    }

    fn remove(&self, file: Ed2kHash, delete_files: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.torrents.remove(&file);
            inner.status.remove(&file);
            inner.removed.push((file, delete_files));
        }
    }

    fn status(&self, file: &Ed2kHash) -> Option<TorrentStatus> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.status.get(file).cloned())
    }

    fn active(&self) -> Vec<Ed2kHash> {
        self.inner
            .lock()
            .map(|inner| inner.torrents.keys().copied().collect())
            .unwrap_or_default()
    }

    fn add_import(&self, id: TorrentImportId, chosen: &NyaaMatch, output_dir: PathBuf) {
        if let Ok(mut inner) = self.inner.lock() {
            inner
                .imports
                .entry(id)
                .or_insert_with(|| (chosen.clone(), output_dir));
        }
    }

    fn remove_import(&self, id: TorrentImportId, delete_files: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.imports.remove(&id);
            inner.import_status.remove(&id);
            inner.removed_imports.push((id, delete_files));
        }
    }

    fn import_status(&self, id: TorrentImportId) -> Option<TorrentStatus> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.import_status.get(&id).cloned())
    }

    fn promote_import(&self, id: TorrentImportId, file: Ed2kHash) {
        if let Ok(mut inner) = self.inner.lock()
            && let Some(value) = inner.imports.remove(&id)
        {
            inner.torrents.insert(file, value);
            if let Some(status) = inner.import_status.remove(&id) {
                inner.status.insert(file, status);
            }
        }
    }

    fn speeds(&self) -> TorrentSpeeds {
        self.inner
            .lock()
            .map(|inner| inner.speeds)
            .unwrap_or_default()
    }

    fn reconcile_session(&self, keep: &HashMap<String, Ed2kHash>) -> Vec<PathBuf> {
        let mut kept_dirs = Vec::new();
        if let Ok(mut inner) = self.inner.lock() {
            for (info_hash, output_dir) in std::mem::take(&mut inner.restored) {
                match keep.get(&info_hash) {
                    Some(&file) => {
                        kept_dirs.push(output_dir.clone());
                        inner.torrents.entry(file).or_insert_with(|| {
                            (
                                NyaaMatch {
                                    title: String::new(),
                                    torrent_url: format!("magnet:?xt=urn:btih:{info_hash}"),
                                    info_hash,
                                },
                                output_dir,
                            )
                        });
                    }
                    None => inner.session_deleted.push(info_hash),
                }
            }
        }
        kept_dirs
    }
}
