//! Properties for the download scheduler's cross-file contract: one
//! shared per-source in-flight budget, walked in strict priority order.
//!
//! Two properties:
//!
//! 1. **Chaos recovery / non-starvation**: after an arbitrary "chaos"
//!    prefix (sources lying about block hashes, sending corrupt chunks,
//!    churning presence, priority and now-playing churn) every active
//!    download must still be recoverable — once every original source
//!    starts behaving honestly and stays present, *every* file must
//!    reach completion with the exact original bytes, whatever priority
//!    order the chaos left in place. A scheduler that starves the tail
//!    of the priority order, or wedges a demoted file, fails here.
//!
//! 2. **Budget bound**: with honest-but-arbitrarily-slow sources and
//!    churning presence/priority, the per-peer in-flight total across
//!    all files never exceeds the shared budget plus the endgame
//!    allowance the code actually grants (see the property's comment
//!    for the bound, stated from `plan_all`/`plan_requests`).
//!
//! This is the liveness counterpart to
//! `dessplay/fuzz/fuzz_targets/download_scheduler.rs`'s crash-safety
//! fuzzing. `Downloads` (`dessplay::download`) has a real history of
//! subtle wedge bugs found by hand, one at a time, after the fact --
//! see its own unit-test regression comments: a stalled block-hash
//! source never re-solicited, a departed driving source stranding a
//! replacement, two sources double-assigned the same chunk in one
//! scheduling pass, a lying source never dropped, a demoted file's
//! stale window stamps bypassing the shared budget. Each was a specific
//! instance of the same underlying questions -- "can chaos ever
//! permanently wedge or starve the scheduler" and "can the budget leak"
//! -- so this generalizes them into properties instead of waiting to
//! hand-write the next regression test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;

use dessplay::download::{DownloadAction, DownloadConfig, Downloads};
use dessplay_core::hash::{Ed2kBlockHash, Ed2kFileHash, ed2k_hash_bytes};
use dessplay_core::net::{Bitfield, CHUNK_SIZE, PeerId, PeerMessage, chunk_count, chunk_range};
use dessplay_core::types::Ed2kHash;
use proptest::prelude::*;

const PEER_NAMES: &[&str] = &["a", "b", "c"];
const N_FILES: usize = 3;
/// 12 chunks per file: bigger than one pipeline (bulk scheduling and
/// the sequential window actually run with `pipeline_depth: 4`), small
/// enough to hash once and write fast across hundreds of cases.
const N_CHUNKS: u32 = 12;
const SIZE_BYTES: u64 = N_CHUNKS as u64 * CHUNK_SIZE;

fn peer(i: usize) -> PeerId {
    PeerId::new(PEER_NAMES[i % PEER_NAMES.len()])
}

fn all_peers() -> Vec<PeerId> {
    (0..PEER_NAMES.len()).map(peer).collect()
}

/// One download's true content and identity. Deterministic bytes, so
/// the (comparatively expensive) ed2k hashing runs once per process.
struct Fx {
    bytes: Vec<u8>,
    hash: Ed2kFileHash,
}

static FIXTURES: LazyLock<Vec<Fx>> = LazyLock::new(|| {
    (0..N_FILES as u8)
        .map(|tag| {
            // Tagged so same-length files get distinct identities.
            let bytes: Vec<u8> = (0..SIZE_BYTES).map(|i| (i % 251) as u8 ^ tag).collect();
            let hash = ed2k_hash_bytes(&bytes);
            Fx { bytes, hash }
        })
        .collect()
});

fn fx_of(file: &Ed2kHash) -> &'static Fx {
    FIXTURES
        .iter()
        .find(|f| f.hash.root == *file)
        .expect("action for an unknown file")
}

/// The six priority permutations of the three files.
const PERMS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

fn priority_order(perm: usize) -> Vec<Ed2kHash> {
    PERMS[perm % PERMS.len()]
        .iter()
        .map(|&i| FIXTURES[i].hash.root)
        .collect()
}

/// `now_playing` choice: one of the three files, or nothing playing.
fn now_playing_of(sel: usize) -> Option<Ed2kHash> {
    (sel < N_FILES).then(|| FIXTURES[sel].hash.root)
}

fn full_bitfield(n_chunks: u32) -> Bitfield {
    let mut bf = Bitfield::new(n_chunks);
    for i in 0..n_chunks {
        bf.set(i);
    }
    bf
}

fn test_config() -> DownloadConfig {
    DownloadConfig {
        pipeline_depth: 4,
        max_sources: 3,
        snub_timeout_millis: 200,
        urgent_age_millis: 150,
    }
}

/// One chaotic round: a peer misbehaves, the source set churns, or the
/// session re-ranks the files.
#[derive(Debug, Clone)]
enum Chaos {
    LieAboutBlockHashes {
        file: usize,
        peer: usize,
    },
    SendCorruptChunk {
        file: usize,
        peer: usize,
        index: u32,
    },
    Advertise {
        file: usize,
        peer: usize,
    },
    Churn {
        file: usize,
        mask: u8,
    },
    Tick {
        millis: u64,
    },
    SetPriority {
        perm: usize,
        now_playing: usize,
    },
}

fn chaos_strategy() -> impl Strategy<Value = Chaos> {
    prop_oneof![
        (0..N_FILES, 0..PEER_NAMES.len())
            .prop_map(|(file, peer)| Chaos::LieAboutBlockHashes { file, peer }),
        (0..N_FILES, 0..PEER_NAMES.len(), 0u32..24)
            .prop_map(|(file, peer, index)| Chaos::SendCorruptChunk { file, peer, index }),
        (0..N_FILES, 0..PEER_NAMES.len()).prop_map(|(file, peer)| Chaos::Advertise { file, peer }),
        (0..N_FILES, 0u8..8).prop_map(|(file, mask)| Chaos::Churn { file, mask }),
        (0u64..2000).prop_map(|millis| Chaos::Tick { millis }),
        (0..PERMS.len(), 0..N_FILES + 1)
            .prop_map(|(perm, now_playing)| Chaos::SetPriority { perm, now_playing }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(150))]

    /// After any chaos prefix, once every source is honest and present,
    /// **every** file completes with its exact bytes — under whatever
    /// priority order and now-playing anchor the chaos left behind.
    /// This is the cross-file non-starvation property: the strict
    /// priority walk plus the shared budget must still hand every file
    /// (including the last-ranked one) enough slots to finish.
    #[test]
    fn chaos_prefix_never_prevents_eventual_completion(
        chaos in prop::collection::vec(chaos_strategy(), 0..40)
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut downloads = Downloads::new(test_config());
        let mut now: u64 = 1000;
        let all = all_peers();
        for (i, f) in FIXTURES.iter().enumerate() {
            let path: PathBuf = dir.path().join(format!("dl{i}.bin"));
            downloads.start(f.hash.root, SIZE_BYTES, f.hash.root, path, all.clone(), 0, now);
        }

        // Chaos prefix: every original source may lie, send corrupt
        // chunks, or vanish and reappear, and the fill order / playback
        // anchor may churn, in any order.
        for event in &chaos {
            now += 1;
            match event {
                Chaos::LieAboutBlockHashes { file, peer: p } => {
                    let f = &FIXTURES[*file];
                    downloads.on_peer_message(
                        peer(*p),
                        PeerMessage::BlockHashes {
                            file: f.hash.root,
                            hashes: vec![Ed2kBlockHash([0xAB; 16]); f.hash.blocks.len()],
                        },
                        now,
                    );
                }
                Chaos::SendCorruptChunk { file, peer: p, index } => {
                    let f = &FIXTURES[*file];
                    downloads.on_peer_message(
                        peer(*p),
                        PeerMessage::ChunkData {
                            file: f.hash.root,
                            index: index % N_CHUNKS,
                            data: vec![0xFF; CHUNK_SIZE as usize],
                        },
                        now,
                    );
                }
                Chaos::Advertise { file, peer: p } => {
                    downloads.on_peer_message(
                        peer(*p),
                        PeerMessage::FileAvailability {
                            file: FIXTURES[*file].hash.root,
                            bitfield: full_bitfield(chunk_count(SIZE_BYTES)),
                        },
                        now,
                    );
                }
                Chaos::Churn { file, mask } => {
                    let sources: Vec<PeerId> = (0..PEER_NAMES.len())
                        .filter(|i| mask & (1 << i) != 0)
                        .map(peer)
                        .collect();
                    downloads.set_sources(FIXTURES[*file].hash.root, sources, 0, now);
                }
                Chaos::Tick { millis } => {
                    now += millis;
                    downloads.tick(now);
                }
                Chaos::SetPriority { perm, now_playing } => {
                    downloads.set_priority(priority_order(*perm), now_playing_of(*now_playing));
                }
            }
        }

        // Honest epilogue: every original source is present again and
        // answers truthfully from here on, for every file. Past any
        // snub timeout (and the solicit-attempt cooldown) so lingering
        // bad state has a chance to clear before we start counting.
        // The priority order and now-playing anchor stay wherever the
        // chaos left them: completion must not depend on them.
        now += 10_000;
        let present: HashSet<String> = PEER_NAMES.iter().map(|s| s.to_string()).collect();
        let mut actions = Vec::new();
        for f in FIXTURES.iter() {
            actions.extend(downloads.set_sources(f.hash.root, all.clone(), 0, now));
        }

        let mut completed: HashMap<Ed2kHash, PathBuf> = HashMap::new();
        for _ in 0..100_000 {
            let mut next = Vec::new();
            for action in &actions {
                match action {
                    DownloadAction::Complete { file, path, .. } => {
                        completed.insert(*file, path.clone());
                    }
                    DownloadAction::Send { to, message } if present.contains(&to.to_string()) => {
                        let to = to.clone();
                        match message {
                            PeerMessage::BlockHashRequest { file } => {
                                let f = fx_of(file);
                                now += 1;
                                next.extend(downloads.on_peer_message(
                                    to.clone(),
                                    PeerMessage::BlockHashes {
                                        file: f.hash.root,
                                        hashes: f.hash.blocks.clone(),
                                    },
                                    now,
                                ));
                                now += 1;
                                next.extend(downloads.on_peer_message(
                                    to,
                                    PeerMessage::FileAvailability {
                                        file: f.hash.root,
                                        bitfield: full_bitfield(chunk_count(SIZE_BYTES)),
                                    },
                                    now,
                                ));
                            }
                            PeerMessage::ChunkRequest { file, chunks } => {
                                let f = fx_of(file);
                                for &c in chunks {
                                    let r = chunk_range(c, SIZE_BYTES);
                                    now += 1;
                                    next.extend(downloads.on_peer_message(
                                        to.clone(),
                                        PeerMessage::ChunkData {
                                            file: f.hash.root,
                                            index: c,
                                            data: f.bytes[r.start as usize..r.end as usize]
                                                .to_vec(),
                                        },
                                        now,
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
                if let DownloadAction::Complete { file, path, .. } = a {
                    completed.insert(*file, path.clone());
                }
            }
            if completed.len() == N_FILES {
                break;
            }
            if next.is_empty() {
                now += 1;
                next = downloads.tick(now);
                if next.is_empty() {
                    break;
                }
            }
            actions = next;
        }

        for f in FIXTURES.iter() {
            let path = completed.get(&f.hash.root).expect(
                "every file must complete once every source is honest and present, \
                 regardless of the chaos prefix and the priority it left behind",
            );
            prop_assert_eq!(
                std::fs::read(path).unwrap(),
                f.bytes.clone(),
                "assembled file must match exactly"
            );
        }
    }
}

/// One round of the honest-but-slow world driving the budget-bound
/// property: sources never lie, but deliver only when told to, and
/// presence / priority churn freely.
#[derive(Debug, Clone)]
enum Slow {
    /// A peer delivers the oldest chunk it owes for a file (if any).
    Deliver { peer: usize, file: usize },
    /// Presence churn for one file's source set.
    Churn { file: usize, mask: u8 },
    /// The clock advances (crosses snub and urgent thresholds).
    Tick { millis: u64 },
    /// The fill order churns. `now_playing` stays `None` throughout —
    /// see the property comment for why the bound is stated for the
    /// nothing-playing regime.
    SetPriority { perm: usize },
}

fn slow_strategy() -> impl Strategy<Value = Slow> {
    prop_oneof![
        // Deliveries weighted up so transfers actually progress into
        // endgame under churn.
        4 => (0..PEER_NAMES.len(), 0..N_FILES)
            .prop_map(|(peer, file)| Slow::Deliver { peer, file }),
        1 => (0..N_FILES, 0u8..8).prop_map(|(file, mask)| Slow::Churn { file, mask }),
        1 => (0u64..500).prop_map(|millis| Slow::Tick { millis }),
        1 => (0..PERMS.len()).prop_map(|perm| Slow::SetPriority { perm }),
    ]
}

/// Mirror of the scheduler's per-(peer, file) outstanding requests,
/// reconstructed **only** from observable actions (requests, cancels,
/// deliveries, presence changes we initiated) — never from its
/// internals — so the bound below is a black-box property.
struct BudgetHarness {
    downloads: Downloads,
    now: u64,
    /// Chunks requested and not yet delivered/cancelled, in request
    /// order (the delivery queue), per (peer, file).
    owed: HashMap<(PeerId, Ed2kHash), Vec<u32>>,
    /// Distinct chunks delivered (and therefore written) per file.
    delivered: HashMap<Ed2kHash, HashSet<u32>>,
    /// Files that reached `Complete`, with where they landed.
    completed: HashMap<Ed2kHash, PathBuf>,
}

impl BudgetHarness {
    /// Process a batch of scheduler actions: mirror requests/cancels,
    /// answer solicitations honestly and immediately (feeding the
    /// resulting actions back through), and check the budget bound
    /// after each fully-mirrored batch. The check must come *after*
    /// the batch's Cancels are applied — a batch pairs the requests it
    /// plans with the cancels/drains that freed the room for them, and
    /// checking mid-batch counts both sides at once.
    fn process(&mut self, actions: Vec<DownloadAction>) -> Result<(), TestCaseError> {
        let mut queue = actions;
        while !queue.is_empty() {
            let mut next = Vec::new();
            for action in queue {
                match action {
                    DownloadAction::Send { to, message } => match message {
                        PeerMessage::ChunkRequest { file, chunks } => {
                            self.owed.entry((to, file)).or_default().extend(chunks);
                        }
                        PeerMessage::Cancel { file, chunks } => {
                            if let Some(q) = self.owed.get_mut(&(to, file)) {
                                q.retain(|c| !chunks.contains(c));
                            }
                        }
                        PeerMessage::BlockHashRequest { file } => {
                            let f = fx_of(&file);
                            self.now += 1;
                            next.extend(self.downloads.on_peer_message(
                                to.clone(),
                                PeerMessage::BlockHashes {
                                    file,
                                    hashes: f.hash.blocks.clone(),
                                },
                                self.now,
                            ));
                            self.now += 1;
                            next.extend(self.downloads.on_peer_message(
                                to,
                                PeerMessage::FileAvailability {
                                    file,
                                    bitfield: full_bitfield(chunk_count(SIZE_BYTES)),
                                },
                                self.now,
                            ));
                        }
                        _ => {}
                    },
                    DownloadAction::Complete { file, path, .. } => {
                        // The scheduler dropped the download's state
                        // (sources and their in-flight included); drop
                        // the mirror too.
                        self.owed.retain(|(_, f), _| *f != file);
                        self.completed.insert(file, path);
                    }
                    DownloadAction::Progress { .. } => {}
                    DownloadAction::Abandon { file, reason } => {
                        prop_assert!(false, "unexpected abandon of {file}: {reason}");
                    }
                }
            }
            // The batch's requests are mirrored: the bound must hold
            // for them right now, not only at the next scheduler call.
            self.check()?;
            queue = next;
        }
        Ok(())
    }

    /// A peer delivers the oldest chunk it owes for `file`, if any.
    fn deliver_one(&mut self, p: &PeerId, file: Ed2kHash) -> Result<(), TestCaseError> {
        let Some(q) = self.owed.get_mut(&(p.clone(), file)) else {
            return Ok(());
        };
        if q.is_empty() {
            return Ok(());
        }
        let c = q.remove(0);
        self.delivered.entry(file).or_default().insert(c);
        let f = fx_of(&file);
        let r = chunk_range(c, SIZE_BYTES);
        self.now += 1;
        let actions = self.downloads.on_peer_message(
            p.clone(),
            PeerMessage::ChunkData {
                file,
                index: c,
                data: f.bytes[r.start as usize..r.end as usize].to_vec(),
            },
            self.now,
        );
        // No check here: the mirror must first apply the Cancels this
        // delivery may carry (cancel-elsewhere), which `process` does.
        self.process(actions)
    }

    /// Presence churn for one file: peers dropped from the set lose
    /// their outstanding requests (the scheduler forgets them without a
    /// Cancel — they're gone), so the mirror forgets them too.
    fn churn(&mut self, file: Ed2kHash, mask: u8) -> Result<(), TestCaseError> {
        let sources: Vec<PeerId> = (0..PEER_NAMES.len())
            .filter(|i| mask & (1 << i) != 0)
            .map(peer)
            .collect();
        let kept: HashSet<PeerId> = sources.iter().cloned().collect();
        self.owed.retain(|(p, f), _| *f != file || kept.contains(p));
        self.now += 1;
        let now = self.now;
        let actions = self.downloads.set_sources(file, sources, 0, now);
        self.process(actions)
    }

    /// The budget bound, stated from the code (`plan_all` /
    /// `plan_requests`), checked after every scheduler call:
    ///
    /// Per peer, summed over files: bulk in-flight never exceeds
    /// `pipeline_depth`, because every commitment (urgent included) is
    /// charged to the shared budget and bulk slots are only granted
    /// while the budget is below the depth. On top of that, *endgame*
    /// urgency (a file whose remaining chunks fit one pipeline) may
    /// duplicate that file's remaining chunks to every source,
    /// bypassing the budget — bounded by the file's needed count, which
    /// endgame caps at `pipeline_depth`. So:
    ///
    /// ```text
    /// in_flight(peer) <= pipeline_depth + Σ needed(f)  over endgame f
    /// ```
    ///
    /// Window-age urgency is the other budget bypass, but it applies
    /// only to the now-playing file, and its overshoot drains without
    /// cancels after a now-playing switch — a per-step bound covering
    /// it would have to track which in-flight chunks were issued while
    /// their file was now-playing. This property therefore drives the
    /// nothing-playing regime (`now_playing: None`, the seeder's and
    /// the paused group's state), where the bound above is exact;
    /// window urgency is pinned by the deterministic unit tests in
    /// `download.rs` and exercised for liveness by the chaos property
    /// above.
    fn check(&self) -> Result<(), TestCaseError> {
        let depth = test_config().pipeline_depth as usize;
        let endgame_allowance: usize = FIXTURES
            .iter()
            .filter(|f| !self.completed.contains_key(&f.hash.root))
            .map(|f| {
                let done = self
                    .delivered
                    .get(&f.hash.root)
                    .map(|d| d.len())
                    .unwrap_or(0);
                let needed = N_CHUNKS as usize - done;
                if needed <= depth { needed } else { 0 }
            })
            .sum();
        for p in all_peers() {
            let total: usize = self
                .owed
                .iter()
                .filter(|((op, _), _)| *op == p)
                .map(|(_, q)| q.len())
                .sum();
            let detail: Vec<String> = self
                .owed
                .iter()
                .filter(|((op, _), q)| *op == p && !q.is_empty())
                .map(|((_, f), q)| {
                    let fi = FIXTURES.iter().position(|x| x.hash.root == *f).unwrap();
                    let done = self.delivered.get(f).map(|d| d.len()).unwrap_or(0);
                    format!("file{fi} (delivered {done}): {q:?}")
                })
                .collect();
            let sched: Vec<String> = FIXTURES
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    format!(
                        "file{i}: {:?}",
                        self.downloads.debug_in_flight(&f.hash.root)
                    )
                })
                .collect();
            prop_assert!(
                total <= depth + endgame_allowance,
                "peer {p} holds {total} outstanding chunks: budget {depth} + endgame allowance \
                 {endgame_allowance} exceeded; owed: {detail:?}; scheduler: {sched:?}"
            );
        }
        Ok(())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(150))]

    /// The shared per-source budget holds at every step: with three
    /// files over one source set, honest-but-arbitrarily-slow sources,
    /// and churning presence and priority, no peer is ever asked for
    /// more than `pipeline_depth` outstanding chunks plus the endgame
    /// allowance (see [`BudgetHarness::check`] for the bound and its
    /// derivation) — and once the sources drain their queues, every
    /// file still completes with exact bytes. Break the budget to
    /// per-file (or forget to walk priority order against the shared
    /// count) and this fails immediately.
    #[test]
    fn per_peer_in_flight_never_exceeds_the_shared_budget_bound(
        rounds in prop::collection::vec(slow_strategy(), 0..60)
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut h = BudgetHarness {
            downloads: Downloads::new(test_config()),
            now: 1000,
            owed: HashMap::new(),
            delivered: HashMap::new(),
            completed: HashMap::new(),
        };
        let all = all_peers();
        let mut initial = Vec::new();
        for (i, f) in FIXTURES.iter().enumerate() {
            let path: PathBuf = dir.path().join(format!("dl{i}.bin"));
            initial.extend(h.downloads.start(
                f.hash.root,
                SIZE_BYTES,
                f.hash.root,
                path,
                all.clone(),
                0,
                h.now,
            ));
        }
        h.process(initial)?;

        for round in &rounds {
            match round {
                Slow::Deliver { peer: p, file } => {
                    h.deliver_one(&peer(*p), FIXTURES[*file].hash.root)?;
                }
                Slow::Churn { file, mask } => {
                    h.churn(FIXTURES[*file].hash.root, *mask)?;
                }
                Slow::Tick { millis } => {
                    h.now += millis;
                    let now = h.now;
                    let actions = h.downloads.tick(now);
                    h.process(actions)?;
                }
                Slow::SetPriority { perm } => {
                    h.downloads.set_priority(priority_order(*perm), None);
                }
            }
        }

        // Epilogue: everyone present for every file, queues drained on
        // demand — the bound must keep holding, and every file must
        // complete (the honest-world liveness check).
        for f in FIXTURES.iter() {
            if !h.completed.contains_key(&f.hash.root) {
                h.churn(f.hash.root, 0b111)?;
            }
        }
        for _ in 0..100_000 {
            if h.completed.len() == N_FILES {
                break;
            }
            let owed_now: Vec<(PeerId, Ed2kHash)> = h
                .owed
                .iter()
                .filter(|(_, q)| !q.is_empty())
                .map(|(k, _)| k.clone())
                .collect();
            if owed_now.is_empty() {
                // Nothing owed: nudge with a tick (snub, re-solicit,
                // urgent sweep, refill), past the snub timeout.
                h.now += 201;
                let now = h.now;
                let actions = h.downloads.tick(now);
                if actions.is_empty() {
                    break;
                }
                h.process(actions)?;
            } else {
                for (p, file) in owed_now {
                    h.deliver_one(&p, file)?;
                }
            }
        }
        for f in FIXTURES.iter() {
            let path = h.completed.get(&f.hash.root).expect(
                "every file must complete once the honest sources drain their queues",
            );
            prop_assert_eq!(
                std::fs::read(path).unwrap(),
                f.bytes.clone(),
                "assembled file must match exactly"
            );
        }
    }
}
