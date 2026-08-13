//! Seeder auto-fetch (Phase 9B): the headless transfer driver.
//!
//! A seeder has no player and never gates — it exists to hold and serve
//! files. This drives its `FileActor` from synced playlist state:
//! resolve every entry against the media roots (so existing copies,
//! including prior downloads in the cache-as-media-root, are served and
//! advertised), and **download everything still missing** from peers
//! that have it (design.md, Seeder Behavior). The `FileActor` serves
//! chunks and block hashes to leechers automatically.
//!
//! Unlike the interactive client's prefetch window, a seeder fetches the
//! *whole* playlist — it is the durable seed.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dessplay_core::net::{PeerId, PeerInfo, PeerMessage, Presence};
use dessplay_core::state::StateView;
use dessplay_core::types::{Ed2kHash, FileAvailability, UserId};
use tokio::sync::mpsc;

use crate::actors::file::{
    FileCommand, FileConfig, FileOutput, Resolution, SCAN_TRANSFER_QUIET_DEFAULT, run,
};
use crate::actors::network::NetworkCommand;
use crate::actors::sync::{Mutation, SyncCommand};

/// Drives a seeder's `FileActor` from playlist state.
pub struct SeederTransfer {
    me: UserId,
    file: mpsc::Sender<FileCommand>,
    sync: mpsc::Sender<SyncCommand>,
    network: mpsc::Sender<NetworkCommand>,
    /// Entries we've kicked a resolve for (once each).
    resolve_kicked: HashSet<Ed2kHash>,
    /// Resolution outcome per entry: `true` = have it (servable).
    have: HashMap<Ed2kHash, bool>,
    /// Library hashes we've already requested AniDB lookups for (the
    /// seeder contributes its library to the group's catalog; dedup so
    /// each hash is requested once per session).
    lookups_requested: HashSet<Ed2kHash>,
}

impl SeederTransfer {
    /// Spawn the file actor and build the driver. The returned receiver
    /// carries file-actor outputs; the caller polls it and feeds each to
    /// [`Self::on_file_output`] (kept separate from `self` so a select
    /// loop can hold it without aliasing the driver).
    pub fn new(
        me: UserId,
        file_config: FileConfig,
        sync: mpsc::Sender<SyncCommand>,
        network: mpsc::Sender<NetworkCommand>,
    ) -> (Self, mpsc::Receiver<FileOutput>) {
        let (file_tx, file_rx) = mpsc::channel(256);
        let (file_out_tx, file_outputs) = mpsc::channel(1024);
        tokio::spawn(run(file_config, file_rx, file_out_tx));
        let transfer = SeederTransfer {
            me,
            file: file_tx,
            sync,
            network,
            resolve_kicked: HashSet::new(),
            have: HashMap::new(),
            lookups_requested: HashSet::new(),
        };
        (transfer, file_outputs)
    }

    /// React to a fresh playlist/peer view: resolve new entries, and
    /// (re)start downloads for everything still missing that a peer has.
    pub async fn on_state(&mut self, view: &StateView, peers: &[PeerInfo]) {
        // Resolve each entry once, in playlist order, to learn whether we
        // already hold it (media roots incl. the cache); the file actor
        // caches the result.
        for entry in &view.playlist {
            let file = entry.hash;
            if self.resolve_kicked.insert(file) {
                let _ = self
                    .file
                    .send(FileCommand::Resolve {
                        file,
                        filename: entry.state.filename.clone(),
                    })
                    .await;
            }
        }
        // Start downloads for everything still missing that a peer has,
        // **unwatched entries first** then in playlist order (design.md,
        // Seeder Behavior): under bandwidth saturation the seeder must
        // finish the next episode the group needs before its watched
        // back-catalog. `StartDownload` is idempotent (refreshes sources).
        for entry in download_order(view) {
            let file = entry.hash;
            if self.have.get(&file) == Some(&false) {
                // Peer sources only: the seeder deliberately runs no
                // torrent path (a torrentable file makes the seeder
                // redundant — design.md, BitTorrent Downloads), so a
                // sourceless emission would just park an empty download.
                let sources = self.sources(view, peers, file);
                if sources.is_empty() {
                    continue;
                }
                let _ = self
                    .file
                    .send(FileCommand::StartDownload {
                        file,
                        size_bytes: entry.state.size_bytes,
                        sources,
                        play_chunk: 0,
                    })
                    .await;
            }
        }
    }

    /// Route a file-actor output: relay messages, availability writes,
    /// and resolution outcomes (which mark what we can serve).
    pub async fn on_file_output(&mut self, output: FileOutput) {
        match output {
            FileOutput::Resolved { file, resolution } => match resolution {
                Resolution::Verified(_) => {
                    self.have.insert(file, true);
                    self.set_availability(file, FileAvailability::Ready).await;
                }
                Resolution::NotFound | Resolution::HashMismatch(_) => {
                    self.have.insert(file, false);
                    // on_state will start the download once a source is
                    // present; advertise Missing meanwhile.
                    self.set_availability(file, FileAvailability::Missing).await;
                }
            },
            FileOutput::SendPeer { to, message } => {
                let _ = self
                    .network
                    .send(NetworkCommand::SendPeer { to, message })
                    .await;
            }
            FileOutput::OpenTransfer { to, file } => {
                let _ = self
                    .network
                    .send(NetworkCommand::OpenTransferStream { to, file })
                    .await;
            }
            FileOutput::Availability { file, availability } => {
                self.set_availability(file, availability).await;
            }
            FileOutput::DownloadComplete { file, .. } => {
                self.have.insert(file, true);
                // The actor already emits Availability::Ready; nothing
                // more to do (no player to load into).
            }
            FileOutput::LibraryIndexed { files } => {
                // The seeder contributes its library to the group's
                // browsable catalog: request a lookup for each new hash
                // (the server dedups against existing metadata/queue and
                // records the file's identity).
                for f in files {
                    if self.lookups_requested.insert(f.hash) {
                        let _ = self
                            .sync
                            .send(SyncCommand::Mutate(Box::new(Mutation::RequestLookup {
                                info: dessplay_core::types::FileHashInfo {
                                    hash: f.hash,
                                    size: f.size,
                                    filename: f.filename,
                                    mtime: Some(f.mtime),
                                    series_hint: f.series_hint,
                                },
                            })))
                            .await;
                    }
                }
            }
            // Hashing/series/placeholder/archive/eviction/scan-progress
            // outputs don't drive a seeder's resolve-and-fetch flow.
            _ => {}
        }
    }

    /// A relayed peer message: hand to the file actor (serve/download).
    pub async fn on_peer(&self, from: PeerId, message: Box<PeerMessage>) {
        let _ = self
            .file
            .send(FileCommand::PeerMessage { from, message })
            .await;
    }

    /// A per-transfer data stream is live: hand it to the file actor
    /// (a seeder both serves and downloads over these).
    pub async fn on_transfer_stream(
        &self,
        peer: PeerId,
        file: Ed2kHash,
        outbound: bool,
        stream: dessplay_core::net::BiStream,
    ) {
        let _ = self
            .file
            .send(FileCommand::TransferStream {
                peer,
                file,
                outbound,
                stream,
            })
            .await;
    }

    /// The network layer answered a stream-open request with failure:
    /// the file actor clears its pending queue and retries on its tick
    /// (the answered-request contract).
    pub async fn on_transfer_stream_failed(&self, peer: PeerId, file: Ed2kHash) {
        let _ = self
            .file
            .send(FileCommand::TransferStreamFailed { peer, file })
            .await;
    }

    /// The control connection died, taking the transfer plane (and any
    /// unanswered stream opens) with it: fail the pending queues.
    pub async fn on_transfer_link_reset(&self) {
        let _ = self.file.send(FileCommand::TransferLinkReset).await;
    }

    /// Present peers (any role) advertising `file` Ready, excluding us.
    fn sources(&self, view: &StateView, peers: &[PeerInfo], file: Ed2kHash) -> Vec<PeerId> {
        peers
            .iter()
            .filter(|p| {
                p.username != self.me
                    && p.presence == Presence::Present
                    && view.file_availability.get(&(p.username.clone(), file))
                        == Some(&FileAvailability::Ready)
            })
            .map(|p| p.username.clone())
            .collect()
    }

    async fn set_availability(&self, file: Ed2kHash, availability: FileAvailability) {
        let _ = self
            .sync
            .send(SyncCommand::Mutate(Box::new(
                Mutation::SetFileAvailability { file, availability },
            )))
            .await;
    }
}

/// The order a seeder starts downloads in: **unwatched entries first**,
/// then in playlist order (design.md, Seeder Behavior). `view.playlist` is
/// already in playlist (display) order; a *stable* sort on the group
/// watched flag (`false` < `true`) keeps that order within each group, so
/// the next unwatched episode is fetched ahead of watched back-catalog
/// when bandwidth is scarce.
fn download_order(view: &StateView) -> Vec<&dessplay_core::playlist::PlaylistEntry> {
    let mut ordered: Vec<_> = view.playlist.iter().collect();
    ordered.sort_by_key(|entry| view.watched.get(&entry.hash).copied().unwrap_or(false));
    ordered
}

/// Build the seeder's [`FileConfig`]: retention is `infinite` and the
/// hash cache persists in `storage`. Prior downloads are re-discovered
/// at startup by the file actor's cache reconciliation (the cache is
/// hash-addressed and resolved by hash). It also scans its media roots
/// once a day, contributing its (large, stable) library to the group's
/// browsable catalog (design.md, Seeder Behavior).
///
/// `download` is supplied by the caller (rather than defaulted here) so
/// flags like `--pipeline-depth` reach the seeder, which downloads the
/// whole playlist and benefits from the same tuning interactive clients
/// get.
pub fn seeder_file_config(
    storage: crate::storage::Storage,
    media_roots: Vec<PathBuf>,
    cache_dir: PathBuf,
    clock: crate::actors::network::Clock,
    upload_limit: Option<u64>,
    download: crate::download::DownloadConfig,
) -> FileConfig {
    FileConfig {
        storage,
        media_roots,
        retention: crate::config::CacheRetention::Infinite,
        cache_dir,
        clock,
        download,
        upload_limit,
        // A seeder's store is large and stable: scan daily, not minutely.
        scan_interval: Some(std::time::Duration::from_secs(24 * 60 * 60)),
        scan_transfer_quiet: SCAN_TRANSFER_QUIET_DEFAULT,
        // No torrent path on a seeder: the Nyaa browse import is an
        // interactive-only feature, and a file nyaa can supply makes
        // the seeder redundant — its job is the rare, peer-only files.
        torrent: None,
        nyaa: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// The seeder downloads the whole playlist, so `--pipeline-depth`
    /// must reach its download config — not be fixed at the default.
    /// (Regression: `seeder_file_config` ignored the flag and always
    /// built `DownloadConfig::default()`.)
    #[test]
    fn seeder_honors_pipeline_depth() {
        let storage = crate::storage::Storage::open_in_memory().unwrap();
        let clock: crate::actors::network::Clock = std::sync::Arc::new(|| 0);
        let download = crate::download::DownloadConfig {
            pipeline_depth: 32,
            ..Default::default()
        };
        let config = seeder_file_config(
            storage,
            vec![],
            PathBuf::from("/tmp/cache"),
            clock,
            None,
            download,
        );
        assert_eq!(config.download.pipeline_depth, 32);
    }

    /// The seeder must fetch unwatched entries before watched ones, in
    /// playlist order within each group (design.md, Seeder Behavior).
    /// Regression: `on_state` iterated plain playlist order, so watched
    /// back-catalog (which sits at earlier positions, since EOF leaves
    /// finished entries in place) could complete before the next episode.
    #[test]
    fn download_order_puts_unwatched_first_then_playlist_order() {
        use dessplay_core::playlist::NewPlaylistEntry;
        use dessplay_core::state::CrdtState;
        use dessplay_core::types::SharedTimestamp;

        const A: dessplay_core::types::ActorId = dessplay_core::types::ActorId(1);
        let hash = |i: u8| Ed2kHash([i; 16]);
        let entry = |i: u8| NewPlaylistEntry {
            hash: hash(i),
            added_by: UserId::new("baughn"),
            filename: format!("ep{i}.mkv"),
            size_bytes: 1000,
            duration_millis: None,
        };

        // Playlist order: 1, 2, 3, 4. Mark 1 and 3 watched.
        let mut state = CrdtState::new();
        for i in 1..=4 {
            state.push_playlist_entry(A, SharedTimestamp(i as u64), entry(i));
        }
        state.set_watched(A, SharedTimestamp(10), hash(1), true);
        state.set_watched(A, SharedTimestamp(11), hash(3), true);
        let view = state.view();

        // Sanity: the raw playlist is watched-history-first (1,2,3,4).
        let raw: Vec<u8> = view.playlist.iter().map(|e| e.hash.0[0]).collect();
        assert_eq!(raw, vec![1, 2, 3, 4]);

        // download_order: unwatched (2,4) first, then watched (1,3), each
        // preserving playlist order.
        let order: Vec<u8> = download_order(&view).iter().map(|e| e.hash.0[0]).collect();
        assert_eq!(order, vec![2, 4, 1, 3]);
    }
}
