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

use crate::actors::file::{FileCommand, FileConfig, FileOutput, Resolution, run};
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
        };
        (transfer, file_outputs)
    }

    /// React to a fresh playlist/peer view: resolve new entries, and
    /// (re)start downloads for everything still missing that a peer has.
    pub async fn on_state(&mut self, view: &StateView, peers: &[PeerInfo]) {
        for entry in &view.playlist {
            let file = entry.hash;
            // Resolve each entry once to learn whether we already hold
            // it (media roots incl. the cache); the file actor caches.
            if self.resolve_kicked.insert(file) {
                let _ = self
                    .file
                    .send(FileCommand::Resolve {
                        file,
                        filename: entry.state.filename.clone(),
                    })
                    .await;
            }
            // Download anything we've found missing, from peers that have
            // it. `StartDownload` is idempotent (refreshes sources).
            if self.have.get(&file) == Some(&false) {
                let sources = self.sources(view, peers, file);
                if !sources.is_empty() {
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
            FileOutput::Availability { file, availability } => {
                self.set_availability(file, availability).await;
            }
            FileOutput::DownloadComplete { file, .. } => {
                self.have.insert(file, true);
                // The actor already emits Availability::Ready; nothing
                // more to do (no player to load into).
            }
            // Hashing/series/placeholder/archive/eviction outputs don't
            // arise for a seeder's resolve-and-fetch flow.
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

/// Build the seeder's [`FileConfig`]: retention is `infinite` and the
/// hash cache persists in `storage`. Prior downloads are re-discovered
/// at startup by the file actor's cache reconciliation (the cache is
/// hash-addressed and resolved by hash) — no media-root scan needed.
pub fn seeder_file_config(
    storage: crate::storage::Storage,
    media_roots: Vec<PathBuf>,
    cache_dir: PathBuf,
    clock: crate::actors::network::Clock,
    upload_limit: Option<u64>,
) -> FileConfig {
    FileConfig {
        storage,
        media_roots,
        retention: crate::config::CacheRetention::Infinite,
        cache_dir,
        clock,
        download: crate::download::DownloadConfig::default(),
        upload_limit,
    }
}
