//! The librqbit-backed [`TorrentEngine`] (design.md, BitTorrent
//! Downloads).
//!
//! One `librqbit::Session` per process, rooted at `<cache>/torrents/`,
//! with librqbit's own JSON persistence + fastresume so completed
//! torrents keep seeding across restarts without a re-check. The file
//! actor re-adds registered torrents at startup (from the `torrents`
//! table, as magnets); a torrent the session already restored comes
//! back instantly as `AlreadyManaged`.
//!
//! `add`/`remove` are fire-and-forget (spawned tasks — the trait is
//! sync); `status` reads librqbit's sync `stats()`. An add that fails
//! (bad URL, unreachable tracker, magnet that never resolves) marks the
//! entry failed, which `status` surfaces as `error: true` for the fetch
//! policy to see on its next poll.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dessplay_core::types::Ed2kHash;
use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, ManagedTorrent, Session, SessionOptions,
    SessionPersistenceConfig,
};

use super::engine::{TorrentEngine, TorrentStatus};
use super::nyaa::NyaaMatch;

/// The production engine: a librqbit session plus the ed2k → torrent
/// bookkeeping the trait needs.
pub struct RqbitEngine {
    session: Arc<Session>,
    /// Shared with the spawned add tasks, which write the resolved
    /// handle (or the failure) back in.
    torrents: Arc<Mutex<HashMap<Ed2kHash, Entry>>>,
}

struct Entry {
    /// Set once the async add completes; `None` while resolving.
    handle: Option<Arc<ManagedTorrent>>,
    /// Where the payload lands (the actor's per-file torrent dir).
    output_dir: PathBuf,
    /// The add failed terminally.
    failed: bool,
    /// Removed while the add was still in flight; the add task deletes
    /// the torrent as soon as it materializes.
    removed: bool,
}

impl RqbitEngine {
    /// Start a session rooted at `torrents_dir` (created on demand),
    /// with persistence under `torrents_dir/.session/` and the upload
    /// cap from the client's `upload_limit` setting.
    pub async fn new(
        torrents_dir: PathBuf,
        upload_limit: Option<u64>,
    ) -> Result<Arc<Self>, String> {
        Self::new_inner(torrents_dir, upload_limit, false).await
    }

    /// Test-only session: no DHT (a live client on the same machine
    /// already holds librqbit's DHT port) and no listener. Public so the
    /// manual live smoke tests can use it; not for production wiring.
    pub async fn new_for_tests(torrents_dir: PathBuf) -> Result<Arc<Self>, String> {
        Self::new_inner(torrents_dir, None, true).await
    }

    async fn new_inner(
        torrents_dir: PathBuf,
        upload_limit: Option<u64>,
        isolated: bool,
    ) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(&torrents_dir)
            .map_err(|e| format!("creating {}: {e}", torrents_dir.display()))?;
        let session = Session::new_with_opts(
            torrents_dir.clone(),
            SessionOptions {
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(torrents_dir.join(".session")),
                }),
                fastresume: true,
                ratelimits: LimitsConfig {
                    upload_bps: upload_limit
                        .and_then(|bps| u32::try_from(bps).ok())
                        .and_then(NonZeroU32::new),
                    download_bps: None,
                },
                disable_dht: isolated,
                // An ephemeral high range; the default listener port may
                // be held by a live client on the same machine.
                listen_port_range: if isolated { Some(49152..65535) } else { None },
                ..Default::default()
            },
        )
        .await
        .map_err(|e| format!("starting torrent session: {e:#}"))?;
        Ok(Arc::new(RqbitEngine {
            session,
            torrents: Arc::new(Mutex::new(HashMap::new())),
        }))
    }

    /// Fetch a `.torrent` file's bytes. Done here rather than by passing
    /// the URL to librqbit: its `AddTorrent::Url` routes any 40-character
    /// string to the magnet parser (its heuristic for a raw hex
    /// infohash) before checking for an http scheme — and every current
    /// nyaa download URL (7-digit id) is exactly 40 characters, so the
    /// URL path always failed and start-up fell to the slower
    /// DHT-metadata magnet route (2026-07-09). Blocking; call from
    /// `spawn_blocking`.
    fn fetch_torrent_bytes(url: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(url)
            .header("User-Agent", "dessplay/1")
            .call()
            .map_err(|e| format!("fetching {url}: {e}"))?;
        response
            .into_body()
            .with_config()
            // .torrent metadata is tens of KB; anything past 16MB is not it.
            .limit(16 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| format!("reading {url}: {e}"))
    }

    /// The largest payload file's absolute path, once metadata is known.
    fn payload_path(entry: &Entry, handle: &ManagedTorrent) -> Option<PathBuf> {
        let metadata = handle.metadata.load();
        let largest = metadata
            .as_ref()?
            .file_infos
            .iter()
            .max_by_key(|f| f.len)?
            .relative_filename
            .clone();
        Some(entry.output_dir.join(largest))
    }
}

impl TorrentEngine for RqbitEngine {
    fn add(&self, file: Ed2kHash, chosen: &NyaaMatch, output_dir: PathBuf) {
        {
            let Ok(mut torrents) = self.torrents.lock() else {
                return;
            };
            if torrents.contains_key(&file) {
                return;
            }
            torrents.insert(
                file,
                Entry {
                    handle: None,
                    output_dir: output_dir.clone(),
                    failed: false,
                    removed: false,
                },
            );
        }
        let session = self.session.clone();
        let torrents = self.torrents.clone();
        let url = chosen.torrent_url.clone();
        let magnet = format!("magnet:?xt=urn:btih:{}", chosen.info_hash);
        // Fire-and-forget: failures surface through `status`.
        tokio::spawn(async move {
            let opts = || AddTorrentOptions {
                output_folder: Some(output_dir.to_string_lossy().into_owned()),
                overwrite: true,
                ..Default::default()
            };
            // A magnet "url" (startup reconciliation re-adds by magnet)
            // goes straight to librqbit; an http(s) URL is fetched here
            // and added as bytes (see `fetch_torrent_bytes` for why the
            // URL is never handed to librqbit directly).
            let direct: Result<librqbit::AddTorrentResponse, String> = if url.starts_with("magnet:")
            {
                session
                    .add_torrent(AddTorrent::Url(url.clone().into()), Some(opts()))
                    .await
                    .map_err(|e| format!("{e:#}"))
            } else {
                let fetch_url = url.clone();
                match tokio::task::spawn_blocking(move || Self::fetch_torrent_bytes(&fetch_url))
                    .await
                {
                    Ok(Ok(bytes)) => session
                        .add_torrent(AddTorrent::TorrentFileBytes(bytes.into()), Some(opts()))
                        .await
                        .map_err(|e| format!("{e:#}")),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(format!("torrent fetch task: {e}")),
                }
            };
            let result = match direct {
                Ok(response) => Ok(response),
                Err(e) => {
                    tracing::warn!(
                        %file, url,
                        "adding torrent by .torrent file failed ({e}); trying magnet"
                    );
                    session
                        .add_torrent(AddTorrent::Url(magnet.into()), Some(opts()))
                        .await
                        .map_err(|e| format!("{e:#}"))
                }
            };
            let handle = result.map(|response| response.into_handle());
            let Ok(mut torrents) = torrents.lock() else {
                return;
            };
            match (torrents.get_mut(&file), handle) {
                (Some(entry), Ok(Some(handle))) if entry.removed => {
                    // Removed while resolving: delete what just landed.
                    torrents.remove(&file);
                    let session = session.clone();
                    tokio::spawn(async move {
                        let _ = session.delete(TorrentIdOrHash::Id(handle.id()), true).await;
                    });
                }
                (Some(entry), Ok(Some(handle))) => entry.handle = Some(handle),
                (Some(entry), Ok(None)) => {
                    // list_only response — can't happen (we never ask for
                    // it), but a missing handle is a failure all the same.
                    tracing::warn!(%file, "torrent add returned no handle");
                    entry.failed = true;
                }
                (Some(entry), Err(e)) => {
                    tracing::warn!(%file, "adding torrent failed: {e}");
                    entry.failed = true;
                }
                (None, Ok(Some(handle))) => {
                    // Entry vanished (removed + already cleaned up).
                    let session = session.clone();
                    tokio::spawn(async move {
                        let _ = session.delete(TorrentIdOrHash::Id(handle.id()), true).await;
                    });
                }
                (None, _) => {}
            }
        });
    }

    fn remove(&self, file: Ed2kHash, delete_files: bool) {
        let handle = {
            let Ok(mut torrents) = self.torrents.lock() else {
                return;
            };
            if let Some(entry) = torrents.get_mut(&file)
                && entry.handle.is_none()
                && !entry.failed
            {
                // Still resolving: flag it; the add task cleans up.
                entry.removed = true;
                return;
            }
            torrents.remove(&file).and_then(|e| e.handle)
        };
        let Some(handle) = handle else {
            return;
        };
        let session = self.session.clone();
        tokio::spawn(async move {
            if let Err(e) = session
                .delete(TorrentIdOrHash::Id(handle.id()), delete_files)
                .await
            {
                tracing::warn!(%file, "deleting torrent: {e:#}");
            }
        });
    }

    fn status(&self, file: &Ed2kHash) -> Option<TorrentStatus> {
        let torrents = self.torrents.lock().ok()?;
        let entry = torrents.get(file)?;
        if entry.failed {
            return Some(TorrentStatus {
                error: true,
                ..TorrentStatus::default()
            });
        }
        let Some(handle) = &entry.handle else {
            // Still resolving the .torrent/magnet: no progress yet.
            return Some(TorrentStatus::default());
        };
        let stats = handle.stats();
        Some(TorrentStatus {
            progress_bytes: stats.progress_bytes,
            finished: stats.finished,
            error: stats.error.is_some()
                || matches!(stats.state, librqbit::TorrentStatsState::Error),
            payload: Self::payload_path(entry, handle),
        })
    }

    fn active(&self) -> Vec<Ed2kHash> {
        self.torrents
            .lock()
            .map(|torrents| torrents.keys().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// The real session starts against a temp dir (persistence folder
    /// created, DHT up) and unknown files report no status. Ignored by
    /// default: it binds sockets and touches the network stack.
    #[tokio::test]
    #[ignore = "binds real sockets; run manually"]
    async fn session_starts_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let engine = RqbitEngine::new(dir.path().join("torrents"), Some(1_000_000))
            .await
            .unwrap();
        assert!(engine.status(&Ed2kHash([0; 16])).is_none());
        assert!(engine.active().is_empty());
        assert!(dir.path().join("torrents/.session").exists());
    }
}
