//! The torrent-engine seam: what the file actor needs from a BitTorrent
//! implementation for the explicit Nyaa browse import, small enough to
//! fake in tests. The production implementation (librqbit) lives in
//! [`super::rqbit`]; construction sites pass `None` to disable the
//! torrent path entirely (tests, seeders, or the setting off).
//!
//! The interface is deliberately poll-based: adds and removes are
//! fire-and-forget, and the actor reads [`TorrentEngine::import_status`]
//! on its existing tick — no callback channel to plumb, and the fake is
//! a plain mutable map.
//!
//! A torrent starts life as an *import* keyed by [`TorrentImportId`]
//! (its ed2k identity is unknown until the payload is hashed) and is
//! re-keyed to its file via [`TorrentEngine::promote_import`] on
//! completion, so eviction and the live-disable path can remove it by
//! the same hash everything else uses. Nothing survives the process:
//! the engine keeps no persistent session (design.md, BitTorrent
//! Downloads — imports seed only for the session that downloaded them).

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
    /// multi-file torrent (unexpected — browse only offers single-file
    /// payloads) the engine reports the largest file.
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
    /// Start a user-selected torrent whose ed2k identity is not known yet.
    /// Idempotent: re-adding a known import is a no-op.
    fn add_import(&self, id: TorrentImportId, chosen: &NyaaMatch, output_dir: PathBuf);
    /// Remove a pending user-selected torrent, deleting its files when
    /// asked. A no-op for an unknown import.
    fn remove_import(&self, id: TorrentImportId, delete_files: bool);
    /// Poll a pending user-selected torrent.
    fn import_status(&self, id: TorrentImportId) -> Option<TorrentStatus>;
    /// Re-key a completed import to its file so eviction and the
    /// live-disable path own its seeding lifecycle.
    fn promote_import(&self, id: TorrentImportId, file: Ed2kHash);
    /// Remove the torrent for `file` (a promoted import), deleting its
    /// files when asked. A no-op for an unknown file.
    fn remove(&self, file: Ed2kHash, delete_files: bool);
    /// Every file the engine has a promoted torrent for (the
    /// live-disable sweep).
    fn active(&self) -> Vec<Ed2kHash>;
    /// Aggregate live up/down speeds across every torrent, for the
    /// status field's bandwidth display. Default: no measurable
    /// traffic.
    fn speeds(&self) -> TorrentSpeeds {
        TorrentSpeeds::default()
    }
}

/// In-memory fake for actor tests: adds and removes record themselves,
/// and tests script status via [`FakeTorrentEngine::set_import_status`].
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
    /// Scripted aggregate speeds ([`TorrentEngine::speeds`]).
    speeds: TorrentSpeeds,
}

impl FakeTorrentEngine {
    /// What a promoted import holds for `file`, if anything.
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

    /// Every `remove_import` call seen, in order.
    pub fn removed_imports(&self) -> Vec<(TorrentImportId, bool)> {
        self.inner
            .lock()
            .map(|inner| inner.removed_imports.clone())
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

    fn remove(&self, file: Ed2kHash, delete_files: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.torrents.remove(&file);
            inner.status.remove(&file);
            inner.removed.push((file, delete_files));
        }
    }

    fn active(&self) -> Vec<Ed2kHash> {
        self.inner
            .lock()
            .map(|inner| inner.torrents.keys().copied().collect())
            .unwrap_or_default()
    }

    fn speeds(&self) -> TorrentSpeeds {
        self.inner
            .lock()
            .map(|inner| inner.speeds)
            .unwrap_or_default()
    }
}
