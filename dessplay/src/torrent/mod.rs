//! Torrent-first downloads (design.md, BitTorrent Downloads).
//!
//! A missing playlist file is fetched via BitTorrent whenever it can be
//! found on nyaa.si; the Phase-9B peer transfer remains the fallback for
//! rare files. This module holds the nyaa search ([`nyaa`]) and the
//! fetch policy core ([`TorrentFetches`]) — the scheduling brain the
//! file actor drives, synchronous and channel-free like
//! [`crate::download::Downloads`] so the policy is deterministic and
//! unit-testable without async or real time.
//!
//! Per-file lifecycle:
//!
//! ```text
//! Searching ──match──▶ Running ──complete──▶ (actor ed2k-verifies)
//!     │no match / error    │stall/engine failure     │mismatch
//!     ▼                    ▼                         ▼
//!  Failed ◀────────────────┴─────── ban infohash ────┘
//!     │ (peer fallback; re-search after cooldown)
//! ```
//!
//! The actor owns the side effects (nyaa HTTP on the blocking pool, the
//! torrent engine, hashing); this core owns *when* — stall watchdog,
//! search cooldown, the banned-infohash memory, and the handoff to the
//! peer-transfer fallback.

pub mod engine;
pub mod nyaa;
pub mod rqbit;

use std::collections::{HashMap, HashSet};

use dessplay_core::net::PeerId;
use dessplay_core::types::Ed2kHash;

use nyaa::NyaaMatch;

/// Tunables for the torrent fetch policy.
#[derive(Clone, Copy, Debug)]
pub struct TorrentFetchConfig {
    /// A running torrent that gains no bytes for this long is declared
    /// stalled: removed and handed to the peer fallback. Generous enough
    /// to cover metadata/tracker startup on a healthy swarm.
    pub stall_timeout_millis: u64,
    /// Minimum spacing between nyaa searches for the same file — the
    /// session re-emits `StartDownload` on every snapshot, and a no-match
    /// result must not re-hit nyaa each time. Doubles as the retry delay
    /// after any failure.
    pub search_cooldown_millis: u64,
    /// A search (one blocking GET) that produces no answer for this long
    /// is treated as failed. Belt-and-braces over the HTTP timeout.
    pub search_timeout_millis: u64,
}

impl Default for TorrentFetchConfig {
    fn default() -> Self {
        TorrentFetchConfig {
            stall_timeout_millis: 90_000,
            search_cooldown_millis: 15 * 60_000,
            search_timeout_millis: 30_000,
        }
    }
}

/// What the policy wants the actor to do.
#[derive(Debug, PartialEq, Eq)]
pub enum TorrentFetchAction {
    /// Run a nyaa search for `filename` on the blocking pool; report the
    /// outcome via [`TorrentFetches::on_searched`].
    Search {
        /// The file.
        file: Ed2kHash,
        /// Exact release filename to query.
        filename: String,
        /// Expected payload size (feeds [`nyaa::pick_match`]'s tolerance).
        size_bytes: u64,
        /// Info hashes to skip (prior downloads failed ed2k verification).
        banned: HashSet<String>,
    },
    /// Hand the accepted match to the torrent engine.
    Add {
        /// The file.
        file: Ed2kHash,
        /// The accepted nyaa result.
        chosen: NyaaMatch,
    },
    /// Remove the torrent from the engine (and delete its files).
    Remove {
        /// The file.
        file: Ed2kHash,
    },
    /// Start (or refresh) the peer-transfer path with the stashed
    /// context. Empty `sources` is fine — it is today's awaiting-source
    /// behavior.
    Fallback {
        /// The file.
        file: Ed2kHash,
        /// File size, for chunk/block geometry.
        size_bytes: u64,
        /// Last-known candidate peers.
        sources: Vec<PeerId>,
        /// Playback chunk anchor.
        play_chunk: u32,
    },
    /// Download progress changed. Honest and uncapped: torrent pieces
    /// arrive out of order, so a partial torrent is *never* playable —
    /// the file actor writes it as the non-playable `Downloading`
    /// availability at any percentage, and the entry flips straight to
    /// Ready once the payload completes and ed2k-verifies (design.md,
    /// BitTorrent Downloads: complete-only playability).
    Progress {
        /// The file.
        file: Ed2kHash,
        /// Downloaded fraction, basis points.
        progress_bps: u16,
    },
}

/// Per-file fetch phase.
enum Phase {
    /// A nyaa search is in flight (on the blocking pool).
    Searching {
        /// When it was dispatched (search timeout watchdog).
        started_at: u64,
    },
    /// The engine is downloading (or fetching metadata for) the torrent.
    Running {
        /// Info hash, for ban bookkeeping on a later verify failure.
        info_hash: String,
        /// Bytes at the last observed progress increase.
        last_progress_bytes: u64,
        /// When bytes last increased (stall watchdog).
        last_progress_at: u64,
        /// Last progress emitted upstream (dedupe).
        last_emitted_bps: Option<u16>,
    },
    /// The payload is complete; the actor is ed2k-hashing it. No
    /// watchdog — hashing is local work that always finishes.
    Verifying {
        /// Info hash, for ban bookkeeping on a verify failure.
        info_hash: String,
    },
    /// The torrent path failed (no match, stall, engine error, or ed2k
    /// mismatch); the peer path has been started. A `StartDownload`
    /// re-emit past `retry_at` re-searches.
    Failed {
        /// When a fresh search becomes allowed.
        retry_at: u64,
    },
}

/// One file's fetch state plus the stashed `StartDownload` context the
/// fallback needs.
struct Fetch {
    phase: Phase,
    filename: String,
    size_bytes: u64,
    sources: Vec<PeerId>,
    play_chunk: u32,
    /// Info hashes whose payload failed ed2k verification for this file;
    /// never picked again (session-lifetime memory).
    banned: HashSet<String>,
}

/// The torrent fetch manager: zero or more files being fetched.
pub struct TorrentFetches {
    config: TorrentFetchConfig,
    files: HashMap<Ed2kHash, Fetch>,
}

impl TorrentFetches {
    /// A manager with the given tunables.
    pub fn new(config: TorrentFetchConfig) -> Self {
        TorrentFetches {
            config,
            files: HashMap::new(),
        }
    }

    /// Whether the torrent path is actively working on `file` (searching,
    /// downloading, or verifying — not a Failed placeholder).
    pub fn is_active(&self, file: &Ed2kHash) -> bool {
        matches!(
            self.files.get(file).map(|f| &f.phase),
            Some(Phase::Searching { .. } | Phase::Running { .. } | Phase::Verifying { .. })
        )
    }

    /// The active torrent's info hash (Running or Verifying).
    pub fn running_info_hash(&self, file: &Ed2kHash) -> Option<&str> {
        match self.files.get(file).map(|f| &f.phase) {
            Some(Phase::Running { info_hash, .. } | Phase::Verifying { info_hash }) => {
                Some(info_hash)
            }
            _ => None,
        }
    }

    /// Files in the Running phase — the ones whose engine status the
    /// actor polls each tick.
    pub fn running_files(&self) -> Vec<Ed2kHash> {
        self.files
            .iter()
            .filter(|(_, f)| matches!(f.phase, Phase::Running { .. }))
            .map(|(file, _)| *file)
            .collect()
    }

    /// A `StartDownload` arrived for `file`. Starts a search, refreshes
    /// the stashed fallback context, re-searches after a failure
    /// cooldown, or keeps the peer path fed while Failed.
    pub fn on_start_download(
        &mut self,
        file: Ed2kHash,
        filename: String,
        size_bytes: u64,
        sources: Vec<PeerId>,
        play_chunk: u32,
        now: u64,
    ) -> Vec<TorrentFetchAction> {
        if let Some(f) = self.files.get_mut(&file) {
            f.filename = filename;
            f.size_bytes = size_bytes;
            f.sources = sources;
            f.play_chunk = play_chunk;
            return match f.phase {
                // Search/download/verify in flight: just the stash refresh.
                Phase::Searching { .. } | Phase::Running { .. } | Phase::Verifying { .. } => {
                    vec![]
                }
                Phase::Failed { retry_at } if now >= retry_at => {
                    f.phase = Phase::Searching { started_at: now };
                    vec![TorrentFetchAction::Search {
                        file,
                        filename: f.filename.clone(),
                        size_bytes: f.size_bytes,
                        banned: f.banned.clone(),
                    }]
                }
                // Still cooling down: keep the peer path fed (its
                // `start` is idempotent and wants source refreshes).
                Phase::Failed { .. } => vec![f.fallback(file)],
            };
        }
        let f = Fetch {
            phase: Phase::Searching { started_at: now },
            filename: filename.clone(),
            size_bytes,
            sources,
            play_chunk,
            banned: HashSet::new(),
        };
        self.files.insert(file, f);
        vec![TorrentFetchAction::Search {
            file,
            filename,
            size_bytes,
            banned: HashSet::new(),
        }]
    }

    /// The nyaa search finished: `found` is the accepted match, or `None`
    /// for no match / a search error. Ignored unless a search is actually
    /// in flight (a late result after a cancel or timeout).
    pub fn on_searched(
        &mut self,
        file: Ed2kHash,
        found: Option<NyaaMatch>,
        now: u64,
    ) -> Vec<TorrentFetchAction> {
        let Some(f) = self.files.get_mut(&file) else {
            return vec![];
        };
        if !matches!(f.phase, Phase::Searching { .. }) {
            return vec![];
        }
        match found {
            Some(chosen) => {
                f.phase = Phase::Running {
                    info_hash: chosen.info_hash.clone(),
                    last_progress_bytes: 0,
                    last_progress_at: now,
                    last_emitted_bps: None,
                };
                vec![TorrentFetchAction::Add { file, chosen }]
            }
            None => self.fail(file, now, /* remove_torrent */ false),
        }
    }

    /// Engine progress for a running torrent (from polled stats).
    pub fn on_progress(
        &mut self,
        file: Ed2kHash,
        progress_bytes: u64,
        now: u64,
    ) -> Vec<TorrentFetchAction> {
        let Some(f) = self.files.get_mut(&file) else {
            return vec![];
        };
        let size_bytes = f.size_bytes;
        let Phase::Running {
            last_progress_bytes,
            last_progress_at,
            last_emitted_bps,
            ..
        } = &mut f.phase
        else {
            return vec![];
        };
        if progress_bytes > *last_progress_bytes {
            *last_progress_bytes = progress_bytes;
            *last_progress_at = now;
        }
        let bps = progress_bytes
            .saturating_mul(10_000)
            .checked_div(size_bytes)
            .unwrap_or(0)
            .min(10_000) as u16;
        if *last_emitted_bps == Some(bps) {
            return vec![];
        }
        *last_emitted_bps = Some(bps);
        vec![TorrentFetchAction::Progress {
            file,
            progress_bps: bps,
        }]
    }

    /// The engine reported the torrent failed (tracker/metadata/IO
    /// error) — also the right call for a completed payload that can't
    /// be read back (unlike a verify *mismatch*, this doesn't ban the
    /// info hash; the release isn't at fault).
    pub fn on_engine_failed(&mut self, file: Ed2kHash, now: u64) -> Vec<TorrentFetchAction> {
        if !matches!(
            self.files.get(&file).map(|f| &f.phase),
            Some(Phase::Running { .. } | Phase::Verifying { .. })
        ) {
            return vec![];
        }
        self.fail(file, now, /* remove_torrent */ true)
    }

    /// The engine finished downloading the payload; the actor is about
    /// to ed2k-hash it. Parks the fetch in Verifying — no stall watchdog
    /// while local hashing runs. Returns whether the transition happened
    /// (false = not Running, e.g. already verifying or cancelled), so
    /// the actor spawns the hash exactly once.
    pub fn on_completed(&mut self, file: Ed2kHash) -> bool {
        let Some(f) = self.files.get_mut(&file) else {
            return false;
        };
        let Phase::Running { info_hash, .. } = &f.phase else {
            return false;
        };
        f.phase = Phase::Verifying {
            info_hash: info_hash.clone(),
        };
        true
    }

    /// The completed payload verified against the file's ed2k root: the
    /// fetch is done, drop its state. (The torrent itself stays in the
    /// engine, seeding, until eviction.)
    pub fn on_verified(&mut self, file: &Ed2kHash) {
        self.files.remove(file);
    }

    /// The completed payload did NOT hash to the file's root (a
    /// mislabeled release): ban the info hash for this file, remove the
    /// torrent + its files, and fall back to peers.
    pub fn on_verify_failed(&mut self, file: Ed2kHash, now: u64) -> Vec<TorrentFetchAction> {
        if let Some(f) = self.files.get_mut(&file)
            && let Phase::Running { info_hash, .. } | Phase::Verifying { info_hash } = &f.phase
        {
            let hash = info_hash.clone();
            f.banned.insert(hash);
        }
        self.fail(file, now, /* remove_torrent */ true)
    }

    /// Stop fetching `file` entirely (a local copy turned up through
    /// another channel, or the entry left the playlist). Removes the
    /// torrent and its partial files when one is running.
    pub fn cancel(&mut self, file: &Ed2kHash) -> Vec<TorrentFetchAction> {
        let Some(f) = self.files.remove(file) else {
            return vec![];
        };
        match f.phase {
            Phase::Running { .. } | Phase::Verifying { .. } => {
                vec![TorrentFetchAction::Remove { file: *file }]
            }
            Phase::Searching { .. } | Phase::Failed { .. } => vec![],
        }
    }

    /// The torrent path was disabled at runtime (design.md, BitTorrent
    /// Downloads: the live toggle): drain every fetch, removing engine
    /// torrents (with their files) where one is running, and hand every
    /// file to the peer path with its stashed context — a Searching
    /// entry's peer download was never started, so the fallback is what
    /// keeps it downloading. The map is left empty; a later re-enable
    /// starts from scratch.
    pub fn disable_all(&mut self) -> Vec<TorrentFetchAction> {
        let mut actions = Vec::new();
        for (file, f) in std::mem::take(&mut self.files) {
            if matches!(f.phase, Phase::Running { .. } | Phase::Verifying { .. }) {
                actions.push(TorrentFetchAction::Remove { file });
            }
            actions.push(f.fallback(file));
        }
        actions
    }

    /// Periodic tick: stall + search-timeout watchdogs.
    pub fn tick(&mut self, now: u64) -> Vec<TorrentFetchAction> {
        let stalled: Vec<(Ed2kHash, bool)> = self
            .files
            .iter()
            .filter_map(|(file, f)| match f.phase {
                Phase::Running {
                    last_progress_at, ..
                } if now.saturating_sub(last_progress_at) >= self.config.stall_timeout_millis => {
                    Some((*file, true))
                }
                Phase::Searching { started_at }
                    if now.saturating_sub(started_at) >= self.config.search_timeout_millis =>
                {
                    Some((*file, false))
                }
                _ => None,
            })
            .collect();
        let mut actions = Vec::new();
        for (file, remove) in stalled {
            tracing::info!(%file, "torrent fetch stalled; falling back to peer transfer");
            actions.extend(self.fail(file, now, remove));
        }
        actions
    }

    /// Common failure path: mark Failed (cooldown), optionally remove the
    /// engine torrent, and start the peer fallback.
    fn fail(&mut self, file: Ed2kHash, now: u64, remove_torrent: bool) -> Vec<TorrentFetchAction> {
        let Some(f) = self.files.get_mut(&file) else {
            return vec![];
        };
        f.phase = Phase::Failed {
            retry_at: now + self.config.search_cooldown_millis,
        };
        let mut actions = Vec::new();
        if remove_torrent {
            actions.push(TorrentFetchAction::Remove { file });
        }
        actions.push(f.fallback(file));
        actions
    }
}

impl Fetch {
    fn fallback(&self, file: Ed2kHash) -> TorrentFetchAction {
        TorrentFetchAction::Fallback {
            file,
            size_bytes: self.size_bytes,
            sources: self.sources.clone(),
            play_chunk: self.play_chunk,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn hash(n: u8) -> Ed2kHash {
        Ed2kHash([n; 16])
    }

    fn peer(n: u8) -> PeerId {
        PeerId::new(format!("peer{n}"))
    }

    fn a_match(info_hash: &str) -> NyaaMatch {
        NyaaMatch {
            title: "Show - 01.mkv".into(),
            torrent_url: "https://nyaa.si/download/1.torrent".into(),
            info_hash: info_hash.into(),
        }
    }

    fn config() -> TorrentFetchConfig {
        TorrentFetchConfig::default()
    }

    fn start(
        t: &mut TorrentFetches,
        file: Ed2kHash,
        sources: Vec<PeerId>,
        now: u64,
    ) -> Vec<TorrentFetchAction> {
        t.on_start_download(file, "Show - 01.mkv".into(), 1_000_000, sources, 0, now)
    }

    #[test]
    fn first_start_searches_and_reemits_do_not() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        let actions = start(&mut t, f, vec![peer(9)], 0);
        assert!(matches!(&actions[..], [TorrentFetchAction::Search { .. }]));
        // Snapshot re-emits while searching: quiet.
        assert!(start(&mut t, f, vec![peer(9)], 100).is_empty());
        assert!(t.is_active(&f));
    }

    #[test]
    fn no_match_falls_back_with_stashed_sources() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![peer(9)], 0);
        let actions = t.on_searched(f, None, 50);
        assert_eq!(
            actions,
            vec![TorrentFetchAction::Fallback {
                file: f,
                size_bytes: 1_000_000,
                sources: vec![peer(9)],
                play_chunk: 0,
            }]
        );
        assert!(!t.is_active(&f));
    }

    /// The live-disable drain: a Running fetch removes its torrent and
    /// falls back; a Searching fetch (peer path never started) falls
    /// back without a Remove; everything is forgotten, so a re-enable
    /// starts clean.
    #[test]
    fn disable_all_removes_running_and_falls_back_everything() {
        let mut t = TorrentFetches::new(config());
        let searching = hash(1);
        let running = hash(2);
        start(&mut t, searching, vec![peer(1)], 0);
        start(&mut t, running, vec![peer(2)], 0);
        t.on_searched(running, Some(a_match("aa")), 10);

        let actions = t.disable_all();
        let removes: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                TorrentFetchAction::Remove { file } => Some(*file),
                _ => None,
            })
            .collect();
        let fallbacks: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                TorrentFetchAction::Fallback { file, sources, .. } => Some((*file, sources.len())),
                _ => None,
            })
            .collect();
        assert_eq!(removes, vec![running], "only the running torrent removes");
        let mut sorted = fallbacks.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec![(searching, 1), (running, 1)],
            "every fetch falls back with its stashed sources"
        );
        assert!(!t.is_active(&searching));
        assert!(!t.is_active(&running));
        assert!(t.running_files().is_empty());
    }

    #[test]
    fn failed_reemit_keeps_peer_path_fed_until_cooldown_then_researches() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![], 0);
        t.on_searched(f, None, 0);
        // Inside the cooldown: fallback refresh, no search.
        let actions = start(&mut t, f, vec![peer(2)], 60_000);
        assert!(matches!(
            &actions[..],
            [TorrentFetchAction::Fallback { sources, .. }] if sources == &vec![peer(2)]
        ));
        // Past the cooldown: a fresh search.
        let actions = start(&mut t, f, vec![peer(2)], config().search_cooldown_millis);
        assert!(matches!(&actions[..], [TorrentFetchAction::Search { .. }]));
    }

    #[test]
    fn match_adds_torrent_and_progress_is_honest() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![], 0);
        let actions = t.on_searched(f, Some(a_match("aa")), 10);
        assert!(matches!(&actions[..], [TorrentFetchAction::Add { .. }]));
        assert_eq!(t.running_info_hash(&f), Some("aa"));
        // 50% downloaded reports 50% — playability is carried by the
        // availability *variant* (always non-playable for a torrent),
        // not by capping the display figure.
        let actions = t.on_progress(f, 500_000, 20);
        assert_eq!(
            actions,
            vec![TorrentFetchAction::Progress {
                file: f,
                progress_bps: 5_000,
            }]
        );
        // Unchanged progress is not re-emitted.
        assert!(t.on_progress(f, 500_000, 30).is_empty());
    }

    #[test]
    fn stall_falls_back_exactly_once() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![peer(3)], 0);
        t.on_searched(f, Some(a_match("aa")), 0);
        t.on_progress(f, 100, 1_000);
        // Progress keeps the watchdog quiet.
        assert!(t.tick(1_000 + config().stall_timeout_millis - 1).is_empty());
        let actions = t.tick(1_000 + config().stall_timeout_millis);
        assert_eq!(
            actions,
            vec![
                TorrentFetchAction::Remove { file: f },
                TorrentFetchAction::Fallback {
                    file: f,
                    size_bytes: 1_000_000,
                    sources: vec![peer(3)],
                    play_chunk: 0,
                },
            ]
        );
        // Once: the next tick is quiet.
        assert!(t.tick(1_000_000).is_empty());
    }

    #[test]
    fn search_timeout_falls_back() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![], 0);
        let actions = t.tick(config().search_timeout_millis);
        assert!(matches!(
            &actions[..],
            [TorrentFetchAction::Fallback { .. }]
        ));
        // The late search result is ignored.
        assert!(
            t.on_searched(f, Some(a_match("aa")), config().search_timeout_millis + 1)
                .is_empty()
        );
    }

    #[test]
    fn verify_failure_bans_the_info_hash_for_the_next_search() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![], 0);
        t.on_searched(f, Some(a_match("aa")), 0);
        let actions = t.on_verify_failed(f, 1_000);
        assert!(matches!(
            &actions[..],
            [
                TorrentFetchAction::Remove { .. },
                TorrentFetchAction::Fallback { .. }
            ]
        ));
        // Past the cooldown, the re-search carries the ban.
        let actions = start(&mut t, f, vec![], 1_000 + config().search_cooldown_millis);
        match &actions[..] {
            [TorrentFetchAction::Search { banned, .. }] => {
                assert!(banned.contains("aa"));
            }
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn verified_completion_drops_state() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![], 0);
        t.on_searched(f, Some(a_match("aa")), 0);
        t.on_verified(&f);
        assert!(!t.is_active(&f));
        // Later engine noise for the file is ignored.
        assert!(t.on_progress(f, 999, 10).is_empty());
        assert!(t.on_engine_failed(f, 10).is_empty());
    }

    #[test]
    fn completion_parks_in_verifying_exactly_once_and_dodges_the_watchdog() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![], 0);
        t.on_searched(f, Some(a_match("aa")), 0);
        assert!(t.on_completed(f), "first completion transitions");
        assert!(!t.on_completed(f), "second is a no-op (hash spawns once)");
        // Verifying is not Running: no polling, and no stall watchdog
        // even while a huge payload hashes.
        assert!(t.running_files().is_empty());
        assert!(t.tick(u64::MAX).is_empty());
        assert!(t.is_active(&f));
        // A verify failure from Verifying still bans the info hash.
        t.on_verify_failed(f, 0);
        let actions = start(&mut t, f, vec![], config().search_cooldown_millis);
        match &actions[..] {
            [TorrentFetchAction::Search { banned, .. }] => assert!(banned.contains("aa")),
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn cancel_while_running_removes_torrent_and_while_searching_is_silent() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![], 0);
        assert!(t.cancel(&f).is_empty());
        assert!(!t.is_active(&f));

        start(&mut t, f, vec![], 0);
        t.on_searched(f, Some(a_match("aa")), 0);
        assert_eq!(t.cancel(&f), vec![TorrentFetchAction::Remove { file: f }]);
        assert!(!t.is_active(&f));
    }

    #[test]
    fn engine_failure_falls_back_and_cools_down() {
        let mut t = TorrentFetches::new(config());
        let f = hash(1);
        start(&mut t, f, vec![peer(4)], 0);
        t.on_searched(f, Some(a_match("aa")), 0);
        let actions = t.on_engine_failed(f, 500);
        assert!(matches!(
            &actions[..],
            [
                TorrentFetchAction::Remove { .. },
                TorrentFetchAction::Fallback { .. }
            ]
        ));
        // Re-emit inside the cooldown: fallback only, no re-search.
        let actions = start(&mut t, f, vec![peer(4)], 600);
        assert!(matches!(
            &actions[..],
            [TorrentFetchAction::Fallback { .. }]
        ));
    }
}
