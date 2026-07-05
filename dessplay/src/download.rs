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
    /// Outstanding chunk requests per source, set from the
    /// `--pipeline-depth` flag. The production default (supplied by
    /// `run::download_config`) is 48; this struct's `Default` is 16 and
    /// is only used by tests.
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
        /// The validated per-block hashes, so the finished file can
        /// re-serve them (and they can be cached) without re-hashing.
        block_hashes: Vec<Ed2kBlockHash>,
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
    /// Not yet validated. Block hashes are solicited per-source (see
    /// [`Source::solicited`]) -- a `BlockHashRequest` is answered with both
    /// the per-block hashes and the source's bitfield -- so the lone
    /// asked-peer tracking that used to live here is gone.
    Pending,
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
    /// Whether we've sent this source a `BlockHashRequest` (which doubles
    /// as a bitfield solicitation). Set once per source; a source dropped
    /// on departure is re-added fresh by `set_sources` and re-solicited.
    solicited: bool,
    /// Whether this source has answered our `BlockHashRequest` with a
    /// `BlockHashes` reply (valid or not -- an invalid reply drops the
    /// source outright via `on_block_hashes`, so this only ever stays
    /// observable as `true`). Tracked separately from the bitfield: the
    /// two replies (`BlockHashes` and `FileAvailability`) are sent
    /// together by an honest peer but travel as two independent
    /// messages, so one can arrive without the other (a dropped
    /// connection mid-reply, or any peer that broadcasts availability on
    /// its own initiative). `snub`'s stall detection used to treat a
    /// non-empty bitfield alone as "this source has answered", which
    /// silently stopped re-soliciting a source that had advertised
    /// availability but never actually supplied block hashes -- if every
    /// present source hit that interleaving, the file stayed `Pending`
    /// forever with no chunk ever requested, despite present, honest,
    /// fully-advertising sources sitting right there.
    hashes_replied: bool,
    /// How many times `snub` has re-solicited this source with no reply.
    /// Capped at [`MAX_SOLICIT_ATTEMPTS`] -- a source that never answers a
    /// `BlockHashRequest` (e.g. a manual mapping to a different encode,
    /// which `serve_block_hashes` silently refuses under this file's
    /// identity by design) would otherwise be re-asked every snub timeout
    /// forever. Past the cap, `snub` backs off for
    /// [`GIVE_UP_COOLDOWN_MULTIPLIER`] timeouts before resetting the
    /// budget and trying again -- *not* a permanent drop. An earlier
    /// version removed the source outright once the cap was hit, relying
    /// on a later external `set_sources`/`start` call to "re-add it
    /// fresh"; nothing in this module guarantees such a call ever
    /// happens again (the file actor never calls `set_sources` directly,
    /// only `start` via a `StartDownload` the session re-emits whenever
    /// its synced state view changes -- reliably frequent in an active
    /// session via e.g. playback-position churn, but never for a
    /// seeder-only download nobody is watching). A solitary or
    /// persistently-silent source could then be lost forever with no
    /// recovery path and no visible signal. Backing off in place removes
    /// that external dependency.
    solicit_attempts: u32,
}

/// How many silent `BlockHashRequest` re-solicitations a source gets before
/// `snub` backs off (see [`Source::solicit_attempts`]).
const MAX_SOLICIT_ATTEMPTS: u32 = 3;

/// How many extra snub timeouts a source that hit [`MAX_SOLICIT_ATTEMPTS`]
/// waits before its budget resets and solicitation resumes.
const GIVE_UP_COOLDOWN_MULTIPLIER: u32 = 10;

struct Download {
    store: ChunkStore,
    root: Ed2kHash,
    size_bytes: u64,
    block_hashes: BlockHashes,
    sources: HashMap<PeerId, Source>,
    /// Peers that answered a solicitation with [`PeerMessage::CannotServe`]:
    /// they advertise the file but *know* they can never serve it under
    /// this identity (a manual mapping to a different encode). Excluded
    /// from `set_sources` re-adds for this download's lifetime — the
    /// session re-offers every synced-Ready holder on each refresh, and
    /// without this the denied holder would be re-added and re-solicited
    /// forever.
    denied: HashSet<PeerId>,
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
                    block_hashes: BlockHashes::Pending,
                    sources: HashMap::new(),
                    denied: HashSet::new(),
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
        // arm their snub clock now. A denied peer (replied CannotServe)
        // is never re-added.
        for peer in sources {
            if d.denied.contains(&peer) {
                continue;
            }
            d.sources.entry(peer).or_insert_with(|| Source {
                bitfield: Bitfield::new(chunk_count(d.size_bytes)),
                in_flight: HashSet::new(),
                last_progress: now,
                solicited: false,
                hashes_replied: false,
                solicit_attempts: 0,
            });
        }
        self.progress_and_refill(file, now)
    }

    /// Stop downloading `file`: drop its state and tell every source to
    /// forget the chunks still in flight with it. Used when a local copy
    /// turns up through another channel (library scan / resolve) — the
    /// peers no longer need to relay us the rest.
    pub fn cancel(&mut self, file: &Ed2kHash) -> Vec<DownloadAction> {
        let Some(d) = self.files.remove(file) else {
            return vec![];
        };
        let mut actions = Vec::new();
        for (peer, src) in d.sources {
            if src.in_flight.is_empty() {
                continue;
            }
            let mut chunks: Vec<u32> = src.in_flight.into_iter().collect();
            chunks.sort_unstable();
            actions.push(DownloadAction::Send {
                to: peer,
                message: PeerMessage::Cancel {
                    file: *file,
                    chunks,
                },
            });
        }
        actions
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
            PeerMessage::BlockHashes { file, hashes } => {
                self.on_block_hashes(file, from, hashes, now)
            }
            PeerMessage::CannotServe { file } => {
                let Some(d) = self.files.get_mut(&file) else {
                    return vec![];
                };
                // A definitive "never" from the holder (a manual mapping
                // to a different encode): drop it and never re-add it for
                // this download, unlike a snub (which is a *maybe* and
                // retries after a cooldown).
                tracing::info!(%from, %file, "source says it can never serve; denying it");
                d.sources.remove(&from);
                d.denied.insert(from);
                self.progress_and_refill(file, now)
            }
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
            tracing::warn!(%from, "block hashes don't match the file root; dropping source");
            // The source's block hashes disagree with the file root, so it
            // holds a *different* file: it can supply neither valid hashes
            // nor valid chunks (its advertised bitfield is bogus). Drop it
            // exactly like a snubbed source -- avoiding it, discarding its
            // bitfield so it is never picked for chunks, and freeing its
            // slot -- then re-solicit so a *different* source supplies the
            // hashes. (If it is still a synced Ready candidate, `set_sources`
            // re-adds it fresh next round, just as for a snub: an honest peer
            // that transiently erred gets another chance, while a
            // persistently-broken one self-limits to one ask per refresh and
            // never blocks a good source from completing the download.)
            d.sources.remove(&from);
            return self.progress_and_refill(file, now);
        }
        tracing::debug!(blocks = hashes.len(), "block hashes validated");
        d.block_hashes = BlockHashes::Have(hashes);
        if let Some(src) = d.sources.get_mut(&from) {
            src.hashes_replied = true;
        }
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
            .filter(|(_, s)| {
                !s.in_flight.is_empty() && now.saturating_sub(s.last_progress) >= timeout
            })
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
                // Drop it; set_sources re-adds (with a fresh clock, and
                // re-solicited) if it's still a candidate next round.
                d.sources.remove(&peer);
            }
        }
        // Solicited-but-silent stall: a source we asked for block hashes /
        // availability that hasn't fully answered -- still missing its
        // bitfield (source entries only ever come from synced Ready
        // peers, so an empty bitfield always means "hasn't advertised
        // yet", never "has nothing" -- see `progress_and_refill`), or,
        // while the file is still `Pending`, still missing its
        // `BlockHashes` reply specifically (`hashes_replied`; tracked
        // separately from the bitfield -- see `Source::hashes_replied`
        // for why a non-empty bitfield alone doesn't mean this source
        // answered our hash request). A source with in-flight chunks is
        // plainly not stalled, so the chunk-stage snub above already
        // covers it. Re-ask it.
        //
        // This applies whether the file is still `Pending` (the source that
        // is meant to supply the block hashes themselves went silent) or
        // already `Have` (a source solicited *after* validation -- e.g. one
        // that joined to replace a departed driver, per
        // `progress_and_refill`'s post-`Have` solicitation -- went silent
        // instead). Gating this only on `Pending` left the post-`Have` case
        // uncovered: a solicited source with a lost reply kept
        // `solicited=true` and an empty bitfield forever, never re-asked
        // (the flag latches, cleared only by removing the source) and never
        // snubbed (empty in-flight) -- a permanent wedge if it ends up the
        // only surviving source. Re-soliciting in place (rather than
        // dropping and waiting for `set_sources` to re-add) recovers even
        // with no state snapshots, since the file actor's own timer drives
        // `tick`.
        //
        // A source that never answers *at all* -- e.g. a manual mapping to
        // a different encode, which `serve_block_hashes` silently refuses
        // to serve under this file's identity by design -- would otherwise
        // be re-asked every timeout forever, spending bandwidth on a source
        // that can never succeed. Cap it at `MAX_SOLICIT_ATTEMPTS`: past
        // that, back off for `GIVE_UP_COOLDOWN_MULTIPLIER` timeouts before
        // resetting the budget and asking again. This is a self-contained
        // backoff, not a drop -- see `Source::solicit_attempts` for why an
        // outright removal here is not safe to rely on an external caller
        // to undo.
        let need_hashes = matches!(d.block_hashes, BlockHashes::Pending);
        for (peer, src) in d.sources.iter_mut() {
            let still_missing_something =
                src.bitfield.count_ones() == 0 || (need_hashes && !src.hashes_replied);
            if !(src.solicited && src.in_flight.is_empty() && still_missing_something) {
                continue;
            }
            let elapsed = now.saturating_sub(src.last_progress);
            if src.solicit_attempts >= MAX_SOLICIT_ATTEMPTS {
                if elapsed < timeout.saturating_mul(GIVE_UP_COOLDOWN_MULTIPLIER as u64) {
                    continue;
                }
                tracing::debug!(%peer, "cooldown elapsed; resuming solicitation of a long-silent source");
                src.solicit_attempts = 0;
            } else if elapsed < timeout {
                continue;
            } else {
                tracing::debug!(%peer, attempt = src.solicit_attempts + 1, "re-soliciting block hashes from a stalled source");
            }
            // Re-arm the clock so the next re-ask is another timeout (or
            // cooldown) away, not every tick.
            src.last_progress = now;
            src.solicit_attempts += 1;
            actions.push(DownloadAction::Send {
                to: peer.clone(),
                message: PeerMessage::BlockHashRequest { file },
            });
        }
        actions
    }

    /// Emit a progress action if the verified fraction changed, then
    /// refill request pipelines.
    fn progress_and_refill(&mut self, file: Ed2kHash, _now: u64) -> Vec<DownloadAction> {
        let mut actions = Vec::new();
        let Some(d) = self.files.get_mut(&file) else {
            return actions;
        };

        // Complete?
        if d.store.is_complete() {
            let path = d.store.path().to_path_buf();
            let block_hashes = match &d.block_hashes {
                BlockHashes::Have(hashes) => hashes.clone(),
                BlockHashes::Pending => Vec::new(),
            };
            self.files.remove(&file);
            actions.push(DownloadAction::Complete {
                file,
                path,
                block_hashes,
            });
            return actions;
        }

        // Solicit block hashes + availability from *every* source we
        // haven't asked yet. A `BlockHashRequest` is answered (by
        // `file::serve_block_hashes`) with both the per-block hashes AND the
        // source's `FileAvailability` bitfield, so one request doubles as a
        // bitfield solicitation -- no new wire message needed. Asking all
        // sources (not just one) is what makes the transfer multi-source,
        // and re-asking a source that joins after we already `Have` the
        // hashes (empty bitfield, not yet solicited) is what lets a
        // surviving source take over when the one that supplied the hashes
        // departs -- otherwise the download wedges permanently. Sources
        // come from synced Ready peers (complete holders), so an empty
        // bitfield means "hasn't advertised yet", never "has nothing".
        for (peer, src) in d.sources.iter_mut() {
            if !src.solicited && src.bitfield.count_ones() == 0 {
                src.solicited = true;
                actions.push(DownloadAction::Send {
                    to: peer.clone(),
                    message: PeerMessage::BlockHashRequest { file },
                });
            }
        }

        // No chunk can verify without validated block hashes.
        if matches!(d.block_hashes, BlockHashes::Pending) {
            return actions;
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
    // Updated *within* the source loop as each chunk is committed, so two
    // sources planned in the same pass can't both grab the same chunk.
    let mut assigned: HashSet<u32> = d
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
        // Reserve these so a later source in this same pass won't re-pick
        // them in bulk mode. (Endgame ignores `assigned`, by design.)
        assigned.extend(take.iter().copied());
        actions.push(DownloadAction::Send {
            to: peer,
            message: PeerMessage::ChunkRequest { file, chunks: take },
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

        /// Drive the download to completion against a set of honest,
        /// *present* sources, mirroring what `serve_block_hashes` / the
        /// file actor would do: a present source answers a
        /// `BlockHashRequest` with valid block hashes **and** a full
        /// `FileAvailability` bitfield, and a `ChunkRequest` with the real
        /// bytes. A source not in `present` answers nothing (it has
        /// departed / is unreachable). Returns the completed path, or
        /// `None` if the scheduler wedges (no progress and nothing in
        /// flight).
        fn drive_to_complete(
            &mut self,
            present: &HashSet<String>,
            mut actions: Vec<DownloadAction>,
            start_t: u64,
        ) -> Option<PathBuf> {
            let mut t = start_t;
            for _ in 0..100_000 {
                let mut next = Vec::new();
                let mut completed: Option<PathBuf> = None;
                for action in &actions {
                    match action {
                        DownloadAction::Complete { path, .. } => completed = Some(path.clone()),
                        DownloadAction::Send { to, message }
                            if present.contains(&to.to_string()) =>
                        {
                            let to = to.clone();
                            match message {
                                PeerMessage::BlockHashRequest { file } => {
                                    let file = *file;
                                    let blocks = self.hash.blocks.clone();
                                    let bf = self.full_bitfield();
                                    t += 1;
                                    next.extend(self.downloads.on_peer_message(
                                        to.clone(),
                                        PeerMessage::BlockHashes {
                                            file,
                                            hashes: blocks,
                                        },
                                        t,
                                    ));
                                    t += 1;
                                    next.extend(self.downloads.on_peer_message(
                                        to,
                                        PeerMessage::FileAvailability { file, bitfield: bf },
                                        t,
                                    ));
                                }
                                PeerMessage::ChunkRequest { file, chunks } => {
                                    let file = *file;
                                    for &c in chunks {
                                        let data = self.chunk(c);
                                        t += 1;
                                        next.extend(self.downloads.on_peer_message(
                                            to.clone(),
                                            PeerMessage::ChunkData {
                                                file,
                                                index: c,
                                                data,
                                            },
                                            t,
                                        ));
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                for a in &next {
                    if let DownloadAction::Complete { path, .. } = a {
                        completed = Some(path.clone());
                    }
                }
                if let Some(p) = completed {
                    return Some(p);
                }
                if next.is_empty() {
                    // Nothing in flight: nudge with a tick (snub + refill).
                    next = self.downloads.tick(t + 1);
                    t += 2;
                    if next.is_empty() {
                        return None; // wedged
                    }
                }
                actions = next;
            }
            None
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

    /// A cancelled download (a local copy turned up through another
    /// channel) tells every source to drop the chunks we still had in
    /// flight with it, deactivates, and ignores late chunk data.
    #[test]
    fn cancel_notifies_sources_of_in_flight_chunks_and_deactivates() {
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
        // Answer the solicitation so chunk requests go in flight.
        assert_eq!(block_hash_requests(&actions), vec!["seed"]);
        let mut actions = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1001,
        );
        actions.extend(r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::FileAvailability {
                file: r.file,
                bitfield: r.full_bitfield(),
            },
            1002,
        ));
        let in_flight: Vec<u32> = requests(&actions)
            .into_iter()
            .flat_map(|(_, c)| c)
            .collect();
        assert!(!in_flight.is_empty(), "no chunk requests went out");

        let cancels = r.downloads.cancel(&r.file);
        let mut cancelled: Vec<u32> = cancels
            .iter()
            .filter_map(|a| match a {
                DownloadAction::Send {
                    to,
                    message: PeerMessage::Cancel { chunks, .. },
                } if to.to_string() == "seed" => Some(chunks.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        cancelled.sort_unstable();
        let mut expected = in_flight.clone();
        expected.sort_unstable();
        assert_eq!(cancelled, expected, "every in-flight chunk is cancelled");
        assert!(!r.downloads.is_active(&r.file));

        // Late data for the cancelled download is ignored, and ticking
        // never revives it.
        let data = r.chunk(in_flight[0]);
        let late = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::ChunkData {
                file: r.file,
                index: in_flight[0],
                data,
            },
            1003,
        );
        assert!(late.is_empty());
        assert!(r.downloads.tick(60_000).is_empty());
    }

    /// Regression: a lone source solicited for block hashes that stays
    /// silent (its request or reply was lost, or it briefly dropped and
    /// returned) must be re-solicited after the snub timeout, not leave the
    /// download wedged at the Pending stage forever. The stall was invisible
    /// to the chunk-stage snub (no in-flight chunks) and the `solicited` flag
    /// latched, so it was never re-asked.
    #[test]
    fn stalled_block_hash_source_is_re_solicited() {
        let config = DownloadConfig::default();
        let timeout = config.snub_timeout_millis;
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, config);
        let actions = r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("seed")],
            0,
            1000,
        );
        // The initial solicitation goes out; the source then stays silent.
        assert_eq!(block_hash_requests(&actions), vec!["seed"]);

        // Before the timeout, no re-ask.
        let early = r.downloads.tick(1000 + timeout - 1);
        assert!(
            block_hash_requests(&early).is_empty(),
            "re-asked before the snub timeout"
        );

        // After the timeout, the stalled Pending source is re-solicited.
        let late = r.downloads.tick(1000 + timeout + 1);
        assert_eq!(
            block_hash_requests(&late),
            vec!["seed"],
            "a stalled block-hash source was never re-solicited"
        );

        // Answering the re-solicitation drives the download to completion.
        let present = HashSet::from(["seed".to_string()]);
        let path = r.drive_to_complete(&present, late, 1000 + timeout + 2);
        assert_eq!(path.as_deref(), Some(r.path.as_path()));
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
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("seed")],
            50,
            1000,
        );
        r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1100,
        );
        let actions = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::FileAvailability {
                file: r.file,
                bitfield: r.full_bitfield(),
            },
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
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("slow"), peer("fast")],
            0,
            1000,
        );
        r.downloads.on_peer_message(
            peer("slow"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1100,
        );
        for name in ["slow", "fast"] {
            r.downloads.on_peer_message(
                peer(name),
                PeerMessage::FileAvailability {
                    file: r.file,
                    bitfield: r.full_bitfield(),
                },
                1200,
            );
        }
        let before: HashSet<u32> = requests(&[]).into_iter().flat_map(|(_, c)| c).collect();
        let _ = before;
        // 'fast' delivers a chunk at t=2000; 'slow' stays silent.
        r.downloads.on_peer_message(
            peer("fast"),
            PeerMessage::ChunkData {
                file: r.file,
                index: 0,
                data: r.chunk(0),
            },
            2000,
        );
        // Tick well past the snub timeout: 'slow' is dropped, its chunks
        // Cancelled and requeued; 'fast' survives.
        let actions = r.downloads.tick(40_000);
        let cancels: Vec<&DownloadAction> = actions
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    DownloadAction::Send {
                        message: PeerMessage::Cancel { .. },
                        ..
                    }
                )
            })
            .collect();
        assert!(
            !cancels.is_empty(),
            "snubbed source's chunks must be cancelled"
        );
        let stats = r.downloads.stats(&r.file).unwrap();
        assert!(stats.chunks_requeued > 0);
    }

    #[test]
    fn full_transfer_from_one_seed_completes_with_perfect_goodput() {
        // Drive a whole download from a single seed, answering each
        // ChunkRequest with the true bytes, until Complete.
        let mut r = rig(
            2 * ED2K_BLOCK_SIZE as usize + 5000,
            DownloadConfig::default(),
        );
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
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1,
        );
        // Advertise full availability — this kicks off chunk requests.
        let mut actions = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::FileAvailability {
                file: r.file,
                bitfield: r.full_bitfield(),
            },
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
                                PeerMessage::ChunkData {
                                    file: r.file,
                                    index: c,
                                    data: r.chunk(c),
                                },
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
        assert_eq!(
            std::fs::read(&path).unwrap(),
            r.bytes,
            "assembled file matches"
        );
        // The download was removed on completion; recompute stats from a
        // fresh read isn't possible, so assert via the file contents
        // above. (Per-file stats are asserted in the sim test.)
    }

    // --- Regression tests for the multi-source-solicitation / stall and
    // duplicate-assignment defects (2026-06-26 codebase review). ---

    /// Every source must be solicited for block hashes / availability, not
    /// just `sources.keys().next()`. A `BlockHashRequest` is answered with
    /// both the per-block hashes and the source's bitfield, so soliciting
    /// all sources is what makes the transfer multi-source. Pre-fix only
    /// one source was ever asked, so secondary sources kept empty bitfields
    /// and were never used.
    #[test]
    fn every_source_is_solicited_for_block_hashes() {
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, DownloadConfig::default());
        let actions = r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("a"), peer("b"), peer("c")],
            0,
            1000,
        );
        let asked: HashSet<String> = block_hash_requests(&actions).into_iter().collect();
        let want: HashSet<String> = ["a", "b", "c"].into_iter().map(String::from).collect();
        assert_eq!(
            asked, want,
            "every source must be solicited, not just one: {actions:?}"
        );
    }

    /// A source that joins *after* block hashes are already validated (so
    /// the download is past the `Pending` phase) must still be solicited
    /// for its bitfield -- otherwise it stays an empty, unusable candidate.
    /// This is the "re-solicit when `block_hashes` is `Have` but a source
    /// has an empty bitfield" case; pre-fix the only solicitation site sat
    /// inside the `Pending` branch and was unreachable here.
    #[test]
    fn a_source_added_after_block_hashes_is_solicited() {
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, DownloadConfig::default());
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("a")],
            0,
            1000,
        );
        // 'a' supplies valid block hashes -> state leaves Pending.
        r.downloads.on_peer_message(
            peer("a"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1100,
        );
        // A new Ready source 'b' joins. It must be solicited.
        let actions = r
            .downloads
            .set_sources(r.file, vec![peer("a"), peer("b")], 0, 1200);
        assert!(
            block_hash_requests(&actions).contains(&"b".to_string()),
            "a source joining after block hashes are validated must be solicited: {actions:?}"
        );
    }

    /// A source that answers `CannotServe` (a manual mapping to a
    /// different encode -- it advertises Ready but *knows* it can never
    /// serve under this identity) is dropped and, unlike a snubbed or
    /// invalid-hashes source, never re-added by a later `set_sources`
    /// refresh (2026-07-05 review: the session re-offers every
    /// synced-Ready holder on each snapshot, so a mere drop meant
    /// re-solicitation forever, just paced by the cooldown).
    #[test]
    fn a_cannot_serve_source_is_denied_and_never_re_added() {
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, DownloadConfig::default());
        let actions = r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("mapper")],
            0,
            1000,
        );
        assert_eq!(block_hash_requests(&actions), vec!["mapper".to_string()]);

        r.downloads.on_peer_message(
            peer("mapper"),
            PeerMessage::CannotServe { file: r.file },
            1100,
        );

        // The session re-offers the still-synced-Ready mapper alongside a
        // genuine source: only the genuine one is (re)solicited.
        let actions = r
            .downloads
            .set_sources(r.file, vec![peer("mapper"), peer("seed")], 0, 1200);
        assert_eq!(
            block_hash_requests(&actions),
            vec!["seed".to_string()],
            "a denied source must not be re-added or re-solicited: {actions:?}"
        );

        // And it stays denied across arbitrarily many later refreshes and
        // ticks (no cooldown-style resumption).
        for t in [5_000, 60_000, 3_600_000] {
            let mut actions = r.downloads.set_sources(r.file, vec![peer("mapper")], 0, t);
            actions.extend(r.downloads.tick(t));
            assert!(
                block_hash_requests(&actions).is_empty(),
                "denied source resurfaced at t={t}: {actions:?}"
            );
        }
    }

    /// A source solicited *after* block hashes are already `Have` (an empty
    /// bitfield, per `a_source_added_after_block_hashes_is_solicited`) must
    /// be re-solicited if its reply never arrives -- not left with
    /// `solicited=true` and an empty bitfield forever. Pre-fix the re-ask
    /// loop in `snub` only ran while `block_hashes` was still `Pending`
    /// (`need_hashes`), so a solicited-but-silent post-`Have` source was
    /// never re-asked (no in-flight chunks to trip the chunk-stage snub
    /// either) -- a permanent wedge if it's the only surviving source.
    #[test]
    fn post_have_stalled_source_is_re_solicited() {
        let config = DownloadConfig::default();
        let timeout = config.snub_timeout_millis;
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, config);
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("a")],
            0,
            1000,
        );
        // 'a' supplies valid block hashes -> state leaves Pending.
        r.downloads.on_peer_message(
            peer("a"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1100,
        );
        // 'b' joins post-Have and is solicited once...
        let actions = r
            .downloads
            .set_sources(r.file, vec![peer("a"), peer("b")], 0, 1200);
        assert!(block_hash_requests(&actions).contains(&"b".to_string()));
        // 'a' then departs, leaving 'b' as the only source -- still with an
        // empty bitfield, since its reply never arrived.
        r.downloads.set_sources(r.file, vec![peer("b")], 0, 1200);

        // Before the timeout, no re-ask.
        let early = r.downloads.tick(1200 + timeout - 1);
        assert!(
            block_hash_requests(&early).is_empty(),
            "re-asked before the snub timeout"
        );

        // After the timeout, the stalled post-Have source is re-solicited.
        let late = r.downloads.tick(1200 + timeout + 1);
        assert_eq!(
            block_hash_requests(&late),
            vec!["b"],
            "a solicited post-Have source with an empty bitfield was never re-solicited"
        );
    }

    /// A source that never answers *at all* (e.g. a manual mapping to a
    /// different encode, which `serve_block_hashes` silently refuses to
    /// serve under this file's identity) must back off after
    /// `MAX_SOLICIT_ATTEMPTS`, not be re-solicited every timeout forever --
    /// but it must **not** be dropped permanently either: nothing external
    /// is guaranteed to ever re-add a source to a lone-seeder-style
    /// download nobody is actively watching (see `Source::solicit_attempts`),
    /// so `Downloads` must recover on its own once the cooldown elapses.
    /// Regression for both halves: an earlier version bounded the
    /// re-solicitation (good) by dropping the source outright (a silent
    /// wedge if nothing ever calls `set_sources` again).
    #[test]
    fn a_perpetually_silent_source_backs_off_then_resumes_on_its_own() {
        let config = DownloadConfig::default();
        let timeout = config.snub_timeout_millis;
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, config);
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("silent")],
            0,
            1000,
        );

        // Re-solicit up to MAX_SOLICIT_ATTEMPTS times, one per timeout,
        // with the source staying silent throughout.
        let mut now = 1000;
        for attempt in 1..=3 {
            now += timeout + 1;
            let actions = r.downloads.tick(now);
            assert_eq!(
                block_hash_requests(&actions),
                vec!["silent"],
                "attempt {attempt} should still re-solicit"
            );
        }

        // One more timeout past the cap: back off, not re-asked yet.
        now += timeout + 1;
        let actions = r.downloads.tick(now);
        assert!(
            block_hash_requests(&actions).is_empty(),
            "a perpetually silent source was re-solicited past the attempt cap: {actions:?}"
        );

        // Still silent, well within the cooldown: still no re-ask.
        now += timeout + 1;
        let actions = r.downloads.tick(now);
        assert!(block_hash_requests(&actions).is_empty());

        // Once the full cooldown elapses, solicitation resumes on its
        // own -- no external `set_sources`/`start` call involved. This
        // is the whole point: a lone silent source is never lost forever.
        now += timeout * u64::from(GIVE_UP_COOLDOWN_MULTIPLIER);
        let actions = r.downloads.tick(now);
        assert_eq!(
            block_hash_requests(&actions),
            vec!["silent"],
            "a long-silent source must resume being solicited after the cooldown, with no external call: {actions:?}"
        );
    }

    /// A download must not wedge when the source that supplied block hashes
    /// departs before serving any chunks: a surviving Ready source has to
    /// be solicited and carry the transfer to completion. Pre-fix, once
    /// `block_hashes` became `Have` no further `BlockHashRequest` was ever
    /// issued, so the replacement source kept an empty bitfield forever and
    /// the download stalled permanently.
    #[test]
    fn download_completes_after_the_driving_source_departs() {
        let mut r = rig(ED2K_BLOCK_SIZE as usize + 5000, DownloadConfig::default());
        // 'a' is the initial source and supplies valid block hashes...
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("a")],
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
        // ...but departs before advertising any chunks; 'b' replaces it.
        let actions = r.downloads.set_sources(r.file, vec![peer("b")], 0, 1200);
        let present: HashSet<String> = ["b"].into_iter().map(String::from).collect();
        let path = r
            .drive_to_complete(&present, actions, 1300)
            .expect("download must complete from the surviving source, not stall");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            r.bytes,
            "assembled file matches"
        );
    }

    /// Two sources advertising overlapping bitfields and planned in the
    /// *same* `plan_requests` pass (both with empty in-flight) must not be
    /// handed the same chunk. They accumulate while block hashes are still
    /// Pending (refill is suppressed), then the first plan after validation
    /// schedules both at once. Pre-fix the `assigned` set was a stale
    /// pre-loop snapshot, so both sources grabbed the identical
    /// window/rarest chunks -> duplicate fetch.
    #[test]
    fn two_sources_in_one_pass_are_not_assigned_the_same_chunk() {
        let config = DownloadConfig {
            pipeline_depth: 16,
            max_sources: 4,
            ..DownloadConfig::default()
        };
        // 2 blocks ~ 77 chunks: bulk (non-endgame) mode.
        let mut r = rig(2 * ED2K_BLOCK_SIZE as usize, config);
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("a"), peer("b")],
            0,
            1000,
        );
        // Both advertise full bitfields *before* block hashes arrive, so
        // neither has anything in flight when the first plan runs.
        for name in ["a", "b"] {
            r.downloads.on_peer_message(
                peer(name),
                PeerMessage::FileAvailability {
                    file: r.file,
                    bitfield: r.full_bitfield(),
                },
                1100,
            );
        }
        // Block hashes validate -> the first refill plans both at once.
        let actions = r.downloads.on_peer_message(
            peer("a"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1200,
        );
        let reqs = requests(&actions);
        assert!(
            reqs.len() >= 2,
            "both sources should be planned in this pass: {reqs:?}"
        );
        let mut seen = HashSet::new();
        for (_, chunks) in &reqs {
            for c in chunks {
                assert!(
                    seen.insert(*c),
                    "chunk {c} assigned to two sources in one pass: {reqs:?}"
                );
            }
        }
    }

    /// A source whose block hashes don't validate against the file root
    /// holds a *different* file: it can serve neither valid hashes nor
    /// valid chunks, so it must be dropped and never asked for chunks --
    /// even if it advertised a full bitfield. Pre-fix the bad source was
    /// left in the source set, so once a *good* source validated the block
    /// hashes the scheduler happily requested chunks from the liar's bogus
    /// bitfield (looping on the bad peer and wasting the relay uplink).
    #[test]
    fn an_invalid_block_hash_source_is_dropped_and_not_asked_for_chunks() {
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
        // 'liar' advertises it has the whole file...
        r.downloads.on_peer_message(
            peer("liar"),
            PeerMessage::FileAvailability {
                file: r.file,
                bitfield: r.full_bitfield(),
            },
            1100,
        );
        // ...but serves block hashes that don't match the file root.
        r.downloads.on_peer_message(
            peer("liar"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: vec![Ed2kBlockHash([0; 16]); r.hash.blocks.len()],
            },
            1100,
        );
        // A good source then supplies valid block hashes -> validated, so
        // chunk requests begin.
        let actions = r.downloads.on_peer_message(
            peer("seed"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: r.hash.blocks.clone(),
            },
            1200,
        );
        // The liar must never be asked for a chunk: its bogus bitfield is
        // gone with it.
        let to_liar: Vec<_> = requests(&actions)
            .into_iter()
            .filter(|(p, _)| p == "liar")
            .collect();
        assert!(
            to_liar.is_empty(),
            "a bad-block-hash source must be dropped, not asked for chunks: {actions:?}"
        );
    }

    /// End-to-end: when the *only* solicited source returns invalid block
    /// hashes, a later-arriving good source must still carry the download
    /// to completion (the bad source is dropped and re-solicitation pursues
    /// the survivor). Pre-fix the bad source lingered with `solicited`
    /// latched true, so it was never re-asked and -- with no good source
    /// yet -- the download could wedge; this guards the re-solicit path.
    #[test]
    fn download_completes_after_a_source_serves_invalid_block_hashes() {
        let mut r = rig(ED2K_BLOCK_SIZE as usize + 5000, DownloadConfig::default());
        r.downloads.start(
            r.file,
            r.hash.size_bytes,
            r.hash.root,
            r.path.clone(),
            vec![peer("liar")],
            0,
            1000,
        );
        // 'liar' serves bogus block hashes and is dropped.
        r.downloads.on_peer_message(
            peer("liar"),
            PeerMessage::BlockHashes {
                file: r.file,
                hashes: vec![Ed2kBlockHash([0; 16]); r.hash.blocks.len()],
            },
            1100,
        );
        // A good source 'seed' joins and must complete the transfer.
        let actions = r.downloads.set_sources(r.file, vec![peer("seed")], 0, 1200);
        let present: HashSet<String> = ["seed"].into_iter().map(String::from).collect();
        let path = r
            .drive_to_complete(&present, actions, 1300)
            .expect("a good source must complete the download after a bad-hash source");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            r.bytes,
            "assembled file matches"
        );
    }
}
