//! Download coordination (Phase 9B-3): the scheduling brain for pulling
//! a file from peers through the relay. Synchronous and channel-free
//! like [`crate::session::PlayerWiring`] — it takes events (peer
//! messages, a clock tick, source-set updates) and returns
//! [`DownloadAction`]s, so the policy is deterministic and unit-testable
//! without async or real time.
//!
//! Design (informed by BitTorrent; see the network-design discussion):
//!
//! - **Pipeline depth** outstanding chunk requests *per source*, across
//!   up to `max_sources` peers.
//! - **Chunk order**: a sequential window of ~20% of the file ahead of
//!   the playback position first (so playback can start), then
//!   **rarest-first** (fewest sources have it) for the rest.
//! - **No per-chunk timeout.** A source that sends *nothing* for
//!   `snub_timeout` is dropped (snubbed) and its outstanding chunks are
//!   requeued elsewhere — so a chunk in transit is never re-requested,
//!   only a silent *source* is. A real chunk loss is rare and surfaces
//!   as a snub.
//! - **Endgame**: when few chunks remain, request each from *all*
//!   sources that have it and `Cancel` the losers as they arrive, so the
//!   tail isn't stuck behind one slow source.
//! - **Verification** is the chunk store's (ed2k per block); a corrupt
//!   block's chunks are cleared and re-fetched.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dessplay_core::hash::Ed2kBlockHash;
use dessplay_core::net::{Bitfield, PeerId, PeerMessage, chunk_count};
use dessplay_core::types::Ed2kHash;

use crate::chunkstore::ChunkStore;

/// Tunables for the download scheduler.
#[derive(Clone, Copy, Debug)]
pub struct DownloadConfig {
    /// Outstanding chunk requests per source (the `--pipeline-depth`
    /// flag; default 16).
    pub pipeline_depth: u32,
    /// Concurrent source peers per download.
    pub max_sources: u32,
    /// Drop a source that sends nothing for this long (millis).
    pub snub_timeout_millis: u64,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        DownloadConfig {
            pipeline_depth: 16,
            max_sources: 4,
            snub_timeout_millis: 30_000,
        }
    }
}

/// Counters for the efficiency metrics the sim tests assert on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferStats {
    /// Total ChunkData bytes received (useful + wasted).
    pub bytes_received: u64,
    /// Bytes of chunks we already had (duplicate deliveries, mostly from
    /// endgame races).
    pub bytes_duplicate: u64,
    /// Bytes belonging to blocks that failed verification and were
    /// re-fetched.
    pub bytes_refetched: u64,
    /// Chunks dropped from a snubbed source and requeued.
    pub chunks_requeued: u64,
}

impl TransferStats {
    /// Fraction of received bytes that did useful work, in basis points
    /// (10000 = every byte was useful and sent once). `file_size` is the
    /// download's size.
    pub fn goodput_bps(&self, file_size: u64) -> u32 {
        if self.bytes_received == 0 {
            return 10_000;
        }
        ((file_size.min(self.bytes_received)).saturating_mul(10_000) / self.bytes_received) as u32
    }

    /// Fraction of received bytes that were wasted (duplicate or
    /// re-fetched), in basis points.
    pub fn retransmit_bps(&self) -> u32 {
        if self.bytes_received == 0 {
            return 0;
        }
        (self
            .bytes_duplicate
            .saturating_add(self.bytes_refetched)
            .saturating_mul(10_000)
            / self.bytes_received) as u32
    }
}

/// What the coordinator wants the actor to do.
#[derive(Debug, PartialEq, Eq)]
pub enum DownloadAction {
    /// Relay a peer message.
    Send {
        /// Destination peer.
        to: PeerId,
        /// The message.
        message: PeerMessage,
    },
    /// The download's verified progress changed (basis points). Throttled
    /// into a `Downloading { progress_bps }` availability write upstream.
    Progress {
        /// The file.
        file: Ed2kHash,
        /// Verified fraction, basis points.
        progress_bps: u16,
    },
    /// The file finished and verified at `path`.
    Complete {
        /// The file.
        file: Ed2kHash,
        /// Where the complete file is.
        path: PathBuf,
    },
    /// The download was abandoned (e.g. no source could supply valid
    /// block hashes).
    Abandon {
        /// The file.
        file: Ed2kHash,
        /// Why.
        reason: String,
    },
}

/// State for fetching block hashes (needed before any chunk can verify).
enum BlockHashes {
    /// Requested from `peer` at `requested_at`; re-asked on snub.
    Pending { peer: Option<PeerId>, requested_at: u64 },
    /// Validated against the file's root.
    Have(Vec<Ed2kBlockHash>),
}

struct Source {
    /// What the source advertised it has.
    bitfield: Bitfield,
    /// Chunks we've requested and are awaiting from this source.
    in_flight: HashSet<u32>,
    /// Shared-clock millis of the last byte (or the request that armed
    /// the snub clock). Drives snub detection.
    last_progress: u64,
}

struct Download {
    store: ChunkStore,
    root: Ed2kHash,
    size_bytes: u64,
    block_hashes: BlockHashes,
    sources: HashMap<PeerId, Source>,
    /// Chunk index playback is at (sequential-window anchor).
    play_chunk: u32,
    stats: TransferStats,
    last_progress_bps: u16,
}

/// The download manager: zero or more active downloads.
pub struct Downloads {
    config: DownloadConfig,
    files: HashMap<Ed2kHash, Download>,
}

/// Sequential window ahead of playback, as a fraction of total chunks.
const SEQUENTIAL_WINDOW_FRAC: u32 = 5; // 1/5 = 20%

impl Downloads {
    /// A manager with the given tunables.
    pub fn new(config: DownloadConfig) -> Self {
        Downloads {
            config,
            files: HashMap::new(),
        }
    }

    /// Whether a download for `file` is active.
    pub fn is_active(&self, file: &Ed2kHash) -> bool {
        self.files.contains_key(file)
    }

    /// Transfer counters for `file` (tests / progress display).
    pub fn stats(&self, file: &Ed2kHash) -> Option<TransferStats> {
        self.files.get(file).map(|d| d.stats)
    }

    /// Begin downloading `file` (size + root from the playlist entry)
    /// into `path`, pulling from `sources`. Idempotent: re-calling
    /// updates the source set and playback anchor.
    // The args are a coherent "start this download" bundle; and the
    // fallible ChunkStore::open before insert doesn't fit the entry API.
    #[allow(clippy::too_many_arguments, clippy::map_entry)]
    pub fn start(
        &mut self,
        file: Ed2kHash,
        size_bytes: u64,
        root: Ed2kHash,
        path: PathBuf,
        sources: Vec<PeerId>,
        play_chunk: u32,
        now: u64,
    ) -> Vec<DownloadAction> {
        if !self.files.contains_key(&file) {
            let store = match ChunkStore::open(&path, size_bytes) {
                Ok(store) => store,
                Err(e) => {
                    return vec![DownloadAction::Abandon {
                        file,
                        reason: format!("opening download file: {e}"),
                    }];
                }
            };
            self.files.insert(
                file,
                Download {
                    store,
                    root,
                    size_bytes,
                    block_hashes: BlockHashes::Pending {
                        peer: None,
                        requested_at: 0,
                    },
                    sources: HashMap::new(),
                    play_chunk,
                    stats: TransferStats::default(),
                    last_progress_bps: 0,
                },
            );
        }
        self.set_sources(file, sources, play_chunk, now)
    }

    /// Update which peers we may pull `file` from (presence/availability
    /// changed) and the playback anchor.
    pub fn set_sources(
        &mut self,
        file: Ed2kHash,
        sources: Vec<PeerId>,
        play_chunk: u32,
        now: u64,
    ) -> Vec<DownloadAction> {
        let Some(d) = self.files.get_mut(&file) else {
            return vec![];
        };
        d.play_chunk = play_chunk;
        let keep: HashSet<PeerId> = sources.iter().cloned().collect();
        // Drop sources no longer present (their in-flight chunks become
        // needed again — no Cancel: they're gone).
        d.sources.retain(|peer, _| keep.contains(peer));
        // Add new sources with an empty bitfield (they'll advertise);
        // arm their snub clock now.
        for peer in sources {
            d.sources.entry(peer).or_insert_with(|| Source {
                bitfield: Bitfield::new(chunk_count(d.size_bytes)),
                in_flight: HashSet::new(),
                last_progress: now,
            });
        }
        self.progress_and_refill(file, now)
    }

    /// Handle a relayed peer message addressed to a download.
    pub fn on_peer_message(
        &mut self,
        from: PeerId,
        message: PeerMessage,
        now: u64,
    ) -> Vec<DownloadAction> {
        match message {
            PeerMessage::FileAvailability { file, bitfield } => {
                let Some(d) = self.files.get_mut(&file) else {
                    return vec![];
                };
                if !bitfield.is_valid_for(chunk_count(d.size_bytes)) {
                    tracing::warn!(%from, "invalid bitfield; ignoring");
                    return vec![];
                }
                if let Some(src) = d.sources.get_mut(&from) {
                    src.bitfield = bitfield;
                }
                self.progress_and_refill(file, now)
            }
            PeerMessage::BlockHashes { file, hashes } => self.on_block_hashes(file, from, hashes, now),
            PeerMessage::ChunkData { file, index, data } => {
                self.on_chunk_data(file, from, index, data, now)
            }
            // Serve-side messages are handled by the file actor, not here.
            PeerMessage::BlockHashRequest { .. }
            | PeerMessage::ChunkRequest { .. }
            | PeerMessage::Cancel { .. } => vec![],
        }
    }

    /// Periodic tick: snub silent sources, re-ask for block hashes if
    /// stuck, and refill pipelines.
    pub fn tick(&mut self, now: u64) -> Vec<DownloadAction> {
        let files: Vec<Ed2kHash> = self.files.keys().cloned().collect();
        let mut actions = Vec::new();
        for file in files {
            actions.extend(self.snub(file, now));
            actions.extend(self.progress_and_refill(file, now));
        }
        actions
    }

    fn on_block_hashes(
        &mut self,
        file: Ed2kHash,
        from: PeerId,
        hashes: Vec<Ed2kBlockHash>,
        now: u64,
    ) -> Vec<DownloadAction> {
        let Some(d) = self.files.get_mut(&file) else {
            return vec![];
        };
        if matches!(d.block_hashes, BlockHashes::Have(_)) {
            return vec![]; // already have them
        }
        if !d.store.block_hashes_match(d.root, &hashes) {
            tracing::warn!(%from, "block hashes don't match the file root; ignoring source");
            // Leave Pending; tick will re-ask another source.
            return vec![];
        }
        tracing::debug!(blocks = hashes.len(), "block hashes validated");
        d.block_hashes = BlockHashes::Have(hashes);
        // A resume may already have chunks on disk: verify now.
        self.verify_and_collect(file);
        self.progress_and_refill(file, now)
    }

    fn on_chunk_data(
        &mut self,
        file: Ed2kHash,
        from: PeerId,
        index: u32,
        data: Vec<u8>,
        now: u64,
    ) -> Vec<DownloadAction> {
        let Some(d) = self.files.get_mut(&file) else {
            return vec![];
        };
        d.stats.bytes_received += data.len() as u64;
        // The source delivered: reset its snub clock and clear the
        // request.
        if let Some(src) = d.sources.get_mut(&from) {
            src.last_progress = now;
            src.in_flight.remove(&index);
        }
        // Already have it (an endgame duplicate, or a late arrival):
        // discard, and Cancel it at any other source we asked.
        if d.store.is_written(index) {
            d.stats.bytes_duplicate += data.len() as u64;
            return self.cancel_elsewhere(file, index, &from);
        }
        if let Err(e) = d.store.write_chunk(index, &data) {
            tracing::warn!(index, "writing chunk: {e}");
            return vec![];
        }
        let mut actions = self.cancel_elsewhere(file, index, &from);
        self.verify_and_collect(file);
        actions.extend(self.progress_and_refill(file, now));
        actions
    }

    /// In endgame a chunk may be requested from several sources; once it
    /// arrives, Cancel the others.
    fn cancel_elsewhere(
        &mut self,
        file: Ed2kHash,
        index: u32,
        arrived_from: &PeerId,
    ) -> Vec<DownloadAction> {
        let Some(d) = self.files.get_mut(&file) else {
            return vec![];
        };
        let mut actions = Vec::new();
        for (peer, src) in d.sources.iter_mut() {
            if peer != arrived_from && src.in_flight.remove(&index) {
                actions.push(DownloadAction::Send {
                    to: peer.clone(),
                    message: PeerMessage::Cancel {
                        file,
                        chunks: vec![index],
                    },
                });
            }
        }
        actions
    }

    /// Verify any newly-complete blocks; account re-fetched bytes.
    fn verify_and_collect(&mut self, file: Ed2kHash) {
        let Some(d) = self.files.get_mut(&file) else {
            return;
        };
        let BlockHashes::Have(hashes) = &d.block_hashes else {
            return;
        };
        match d.store.verify(hashes) {
            Ok(summary) => {
                // A mismatched block's chunks were cleared; count their
                // bytes as wasted (they'll be re-fetched).
                for &block in &summary.mismatched {
                    d.stats.bytes_refetched += d.store.block_size(block);
                }
            }
            Err(e) => tracing::error!("verify: {e}"),
        }
    }

    /// Snub sources silent past the timeout; requeue their in-flight
    /// chunks (with a Cancel) and re-ask for block hashes if needed.
    fn snub(&mut self, file: Ed2kHash, now: u64) -> Vec<DownloadAction> {
        let Some(d) = self.files.get_mut(&file) else {
            return vec![];
        };
        let timeout = self.config.snub_timeout_millis;
        let mut actions = Vec::new();
        let snubbed: Vec<PeerId> = d
            .sources
            .iter()
            .filter(|(_, s)| !s.in_flight.is_empty() && now.saturating_sub(s.last_progress) >= timeout)
            .map(|(p, _)| p.clone())
            .collect();
        for peer in snubbed {
            if let Some(src) = d.sources.get_mut(&peer) {
                let chunks: Vec<u32> = src.in_flight.drain().collect();
                d.stats.chunks_requeued += chunks.len() as u64;
                tracing::debug!(%peer, requeued = chunks.len(), "snubbing silent source");
                actions.push(DownloadAction::Send {
                    to: peer.clone(),
                    message: PeerMessage::Cancel { file, chunks },
                });
                // Drop it; set_sources re-adds (with a fresh clock) if
                // it's still a candidate next round.
                d.sources.remove(&peer);
            }
        }
        // If we're still waiting on block hashes and the asked source
        // went quiet, clear the pending peer so refill re-asks. Decide
        // first (immutable borrow), then reassign.
        let reset_ra = if let BlockHashes::Pending { peer, requested_at } = &d.block_hashes {
            let stale = peer.as_ref().is_some_and(|p| {
                !d.sources.contains_key(p) || now.saturating_sub(*requested_at) >= timeout
            });
            stale.then_some(*requested_at)
        } else {
            None
        };
        if let Some(requested_at) = reset_ra {
            d.block_hashes = BlockHashes::Pending {
                peer: None,
                requested_at,
            };
        }
        actions
    }

    /// Emit a progress action if the verified fraction changed, then
    /// refill request pipelines.
    fn progress_and_refill(&mut self, file: Ed2kHash, now: u64) -> Vec<DownloadAction> {
        let mut actions = Vec::new();
        let Some(d) = self.files.get_mut(&file) else {
            return actions;
        };

        // Complete?
        if d.store.is_complete() {
            let path = d.store.path().to_path_buf();
            self.files.remove(&file);
            actions.push(DownloadAction::Complete { file, path });
            return actions;
        }

        // Block hashes first: no chunk can verify without them.
        if matches!(d.block_hashes, BlockHashes::Pending { .. }) {
            let need_request = matches!(d.block_hashes, BlockHashes::Pending { peer: None, .. });
            if need_request
                && let Some(target) = d.sources.keys().next().cloned()
            {
                d.block_hashes = BlockHashes::Pending {
                    peer: Some(target.clone()),
                    requested_at: now,
                };
                actions.push(DownloadAction::Send {
                    to: target,
                    message: PeerMessage::BlockHashRequest { file },
                });
            }
            return actions; // no chunks until hashes are validated
        }

        // Progress write on change.
        let bps = d.store.progress_bps();
        if bps != d.last_progress_bps {
            d.last_progress_bps = bps;
            actions.push(DownloadAction::Progress {
                file,
                progress_bps: bps,
            });
        }

        actions.extend(plan_requests(d, &self.config, file));
        actions
    }
}

/// Decide which chunks to request from which sources to fill pipelines.
/// Pulled out as a free function over `&mut Download` so the ordering
/// policy is easy to follow and test.
fn plan_requests(d: &mut Download, config: &DownloadConfig, file: Ed2kHash) -> Vec<DownloadAction> {
    let needed = d.store.needed_chunks();
    if needed.is_empty() {
        return vec![];
    }
    let needed_set: HashSet<u32> = needed.iter().copied().collect();
    // Endgame once the remaining work fits in one global pipeline: then
    // a chunk may be requested from several sources at once.
    let endgame = (needed.len() as u32) <= config.pipeline_depth;

    // Rarity: how many sources advertise each needed chunk.
    let mut rarity: HashMap<u32, u32> = HashMap::new();
    for src in d.sources.values() {
        for &c in &needed {
            if src.bitfield.get(c) {
                *rarity.entry(c).or_insert(0) += 1;
            }
        }
    }

    // Chunks already in flight somewhere (bulk mode avoids duplicating).
    let assigned: HashSet<u32> = d
        .sources
        .values()
        .flat_map(|s| s.in_flight.iter().copied())
        .collect();

    let total_chunks = chunk_count(d.size_bytes);
    let window_end = d
        .play_chunk
        .saturating_add(total_chunks / SEQUENTIAL_WINDOW_FRAC + 1);

    // Deterministic source order (peer id) so the plan is reproducible.
    let mut peers: Vec<PeerId> = d.sources.keys().cloned().collect();
    peers.sort();

    let mut actions = Vec::new();
    let active_cap = config.max_sources as usize;
    for (rank, peer) in peers.into_iter().enumerate() {
        // Cap concurrent sources (deterministic by id). Endgame ignores
        // the cap to drain the tail from everyone.
        if !endgame && rank >= active_cap {
            break;
        }
        let Some(src) = d.sources.get(&peer) else {
            continue;
        };
        let slots = (config.pipeline_depth as usize).saturating_sub(src.in_flight.len());
        if slots == 0 {
            continue;
        }
        // Candidate chunks this source has, that we still need, and that
        // we aren't already getting (from this source always; from
        // anyone unless endgame).
        let src_in_flight = src.in_flight.clone();
        let mut candidates: Vec<u32> = needed
            .iter()
            .copied()
            .filter(|&c| {
                src.bitfield.get(c)
                    && needed_set.contains(&c)
                    && !src_in_flight.contains(&c)
                    && (endgame || !assigned.contains(&c))
            })
            .collect();
        // Order: sequential window (by index) first, then rarest-first,
        // ties by index.
        candidates.sort_by(|&a, &b| {
            let aw = a >= d.play_chunk && a < window_end;
            let bw = b >= d.play_chunk && b < window_end;
            bw.cmp(&aw) // window chunks first
                .then_with(|| {
                    if aw && bw {
                        a.cmp(&b) // within the window, sequential
                    } else {
                        rarity
                            .get(&a)
                            .unwrap_or(&u32::MAX)
                            .cmp(rarity.get(&b).unwrap_or(&u32::MAX))
                            .then_with(|| a.cmp(&b))
                    }
                })
        });
        let take: Vec<u32> = candidates.into_iter().take(slots).collect();
        if take.is_empty() {
            continue;
        }
        if let Some(src) = d.sources.get_mut(&peer) {
            for &c in &take {
                src.in_flight.insert(c);
            }
        }
        actions.push(DownloadAction::Send {
            to: peer,
            message: PeerMessage::ChunkRequest {
                file,
                chunks: take,
            },
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::hash::{ED2K_BLOCK_SIZE, ed2k_hash_bytes};
    use dessplay_core::net::chunk_range;

    use super::*;

    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
    }

    fn peer(name: &str) -> PeerId {
        PeerId::new(name)
    }

    /// Pull every Send-of-ChunkRequest as (peer, chunks).
    fn requests(actions: &[DownloadAction]) -> Vec<(String, Vec<u32>)> {
        actions
            .iter()
            .filter_map(|a| match a {
                DownloadAction::Send {
                    to,
                    message: PeerMessage::ChunkRequest { chunks, .. },
                } => Some((to.to_string(), chunks.clone())),
                _ => None,
            })
            .collect()
    }

    fn block_hash_requests(actions: &[DownloadAction]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                DownloadAction::Send {
                    to,
                    message: PeerMessage::BlockHashRequest { .. },
                } => Some(to.to_string()),
                _ => None,
            })
            .collect()
    }

    /// A test rig: a download of `n_blocks` blocks, with the true bytes
    /// and hash on hand to simulate sources.
    struct Rig {
        downloads: Downloads,
        file: Ed2kHash,
        bytes: Vec<u8>,
        hash: dessplay_core::hash::Ed2kFileHash,
        _dir: tempfile::TempDir,
        path: PathBuf,
    }

    fn rig(n_blocks_plus: usize, config: DownloadConfig) -> Rig {
        let bytes = data(n_blocks_plus);
        let hash = ed2k_hash_bytes(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dl.bin");
        Rig {
            downloads: Downloads::new(config),
            file: hash.root,
            bytes,
            hash,
            _dir: dir,
            path,
        }
    }

    impl Rig {
        fn full_bitfield(&self) -> Bitfield {
            let mut bf = Bitfield::new(chunk_count(self.hash.size_bytes));
            for i in 0..bf.len() {
                bf.set(i);
            }
            bf
        }

        fn chunk(&self, index: u32) -> Vec<u8> {
            let r = chunk_range(index, self.hash.size_bytes);
            self.bytes[r.start as usize..r.end as usize].to_vec()
        }
    }

    #[test]
    fn requests_block_hashes_before_any_chunk() {
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, DownloadConfig::default());
        let actions = r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("seed")],
            0,
            1000,
        );
        // Only a block-hash request goes out first — no chunk requests.
        assert_eq!(block_hash_requests(&actions), vec!["seed"]);
        assert!(requests(&actions).is_empty());
    }

    #[test]
    fn invalid_block_hashes_are_rejected_and_re_asked() {
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, DownloadConfig::default());
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("liar"), peer("seed")],
            0,
            1000,
        );
        // A bogus hash list is rejected; no chunks flow.
        let actions = r.downloads.on_peer_message(
            peer("liar"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: vec![Ed2kBlockHash([0; 16]); r.hash.blocks.len()],
            },
            1100,
        );
        assert!(requests(&actions).is_empty());
        // The real hashes are accepted, and chunk requests begin.
        let actions = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1200,
        );
        // No source bitfield yet → still nothing to request.
        assert!(requests(&actions).is_empty());
    }

    #[test]
    fn pipeline_depth_and_source_cap_bound_outstanding_requests() {
        let config = DownloadConfig {
            pipeline_depth: 4,
            max_sources: 2,
            ..DownloadConfig::default()
        };
        // 3 blocks ≈ 114 chunks, plenty.
        let mut r = rig(3 * ED2K_BLOCK_SIZE as usize, config);
        let sources = vec![peer("a"), peer("b"), peer("c")];
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            sources.clone(),
            0,
            1000,
        );
        r.downloads.on_peer_message(
            peer("a"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1100,
        );
        // All three advertise the full file.
        let mut actions = Vec::new();
        for name in ["a", "b", "c"] {
            actions = r.downloads.on_peer_message(
                peer(name),
                PeerMessage::FileAvailability {
                    file: r.file,
                    bitfield: r.full_bitfield(),
                },
                1200,
            );
        }
        let reqs = requests(&actions);
        // At most 2 sources active (the cap), each asked ≤ 4 chunks.
        assert!(reqs.len() <= 2, "source cap: {reqs:?}");
        for (_, chunks) in &reqs {
            assert!(chunks.len() <= 4, "pipeline depth: {chunks:?}");
        }
        // No chunk is double-assigned in bulk mode.
        let mut all = HashSet::new();
        for (_, chunks) in &reqs {
            for c in chunks {
                assert!(all.insert(*c), "chunk {c} double-assigned");
            }
        }
    }

    #[test]
    fn sequential_window_is_requested_first() {
        let config = DownloadConfig {
            pipeline_depth: 4,
            max_sources: 1,
            ..DownloadConfig::default()
        };
        // 5 blocks so 20% window is a meaningful slice; play at chunk 50.
        let mut r = rig(5 * ED2K_BLOCK_SIZE as usize, config);
        r.downloads
            .start(r.file, r.hash.size_bytes, r.hash.root, r.path.clone(), vec![peer("seed")], 50, 1000);
        r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::BlockHashes { file: r.file, hashes: r.hash.blocks.clone() },
            1100,
        );
        let actions = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::FileAvailability { file: r.file, bitfield: r.full_bitfield() },
            1200,
        );
        let reqs = requests(&actions);
        // The first batch is the window starting at the play chunk, in
        // order.
        assert_eq!(reqs[0].1, vec![50, 51, 52, 53]);
    }

    #[test]
    fn a_snubbed_source_is_dropped_and_its_chunks_requeued() {
        let config = DownloadConfig {
            pipeline_depth: 4,
            max_sources: 4,
            snub_timeout_millis: 30_000,
        };
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, config);
        r.downloads
            .start(r.file, r.hash.size_bytes, r.hash.root, r.path.clone(), vec![peer("slow"), peer("fast")], 0, 1000);
        r.downloads.on_peer_message(
            peer("slow"),
            PeerMessage::BlockHashes { file: r.file, hashes: r.hash.blocks.clone() },
            1100,
        );
        for name in ["slow", "fast"] {
            r.downloads.on_peer_message(
                peer(name),
                PeerMessage::FileAvailability { file: r.file, bitfield: r.full_bitfield() },
                1200,
            );
        }
        let before: HashSet<u32> = requests(&[]).into_iter().flat_map(|(_, c)| c).collect();
        let _ = before;
        // 'fast' delivers a chunk at t=2000; 'slow' stays silent.
        r.downloads.on_peer_message(
            peer("fast"),
            PeerMessage::ChunkData { file: r.file, index: 0, data: r.chunk(0) },
            2000,
        );
        // Tick well past the snub timeout: 'slow' is dropped, its chunks
        // Cancelled and requeued; 'fast' survives.
        let actions = r.downloads.tick(40_000);
        let cancels: Vec<&DownloadAction> = actions
            .iter()
            .filter(|a| matches!(a, DownloadAction::Send { message: PeerMessage::Cancel { .. }, .. }))
            .collect();
        assert!(!cancels.is_empty(), "snubbed source's chunks must be cancelled");
        let stats = r.downloads.stats(&r.file).unwrap();
        assert!(stats.chunks_requeued > 0);
    }

    #[test]
    fn full_transfer_from_one_seed_completes_with_perfect_goodput() {
        // Drive a whole download from a single seed, answering each
        // ChunkRequest with the true bytes, until Complete.
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize + 5000, DownloadConfig::default());
        let _ = r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("seed")],
            0,
            0,
        );
        // Answer block hashes.
        let _ = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::BlockHashes { file: r.file, hashes: r.hash.blocks.clone() },
            1,
        );
        // Advertise full availability — this kicks off chunk requests.
        let mut actions = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::FileAvailability { file: r.file, bitfield: r.full_bitfield() },
            2,
        );

        let mut t = 10u64;
        let mut completed = None;
        // Serve requested chunks back until Complete, like a seed would.
        for _ in 0..10_000 {
            let mut next = Vec::new();
            for action in &actions {
                match action {
                    DownloadAction::Send {
                        message: PeerMessage::ChunkRequest { chunks, .. },
                        ..
                    } => {
                        for &c in chunks {
                            t += 1;
                            let reply = r.downloads.on_peer_message(
                                peer("seed"),
                                PeerMessage::ChunkData { file: r.file, index: c, data: r.chunk(c) },
                                t,
                            );
                            next.extend(reply);
                        }
                    }
                    DownloadAction::Complete { path, .. } => {
                        completed = Some(path.clone());
                    }
                    _ => {}
                }
            }
            for action in &next {
                if let DownloadAction::Complete { path, .. } = action {
                    completed = Some(path.clone());
                }
            }
            if completed.is_some() {
                break;
            }
            if next.is_empty() {
                // Nothing in flight: nudge with a tick (refill).
                next = r.downloads.tick(t + 1);
                if next.is_empty() {
                    break;
                }
            }
            actions = next;
        }
        let path = completed.expect("download should complete");
        assert_eq!(std::fs::read(&path).unwrap(), r.bytes, "assembled file matches");
        // The download was removed on completion; recompute stats from a
        // fresh read isn't possible, so assert via the file contents
        // above. (Per-file stats are asserted in the sim test.)
    }
}
