//! Property: after an arbitrary "chaos" prefix (sources lying about
//! block hashes, sending corrupt chunks, or churning presence) the
//! download must always still be recoverable -- once every original
//! source starts behaving honestly and stays present, the transfer must
//! reach completion with the exact original bytes.
//!
//! This is the liveness counterpart to
//! `dessplay/fuzz/fuzz_targets/download_scheduler.rs`'s crash-safety
//! fuzzing. `Downloads` (`dessplay::download`) has a real history of
//! subtle wedge bugs found by hand, one at a time, after the fact --
//! see its own unit-test regression comments: a stalled block-hash
//! source never re-solicited, a departed driving source stranding a
//! replacement, two sources double-assigned the same chunk in one
//! scheduling pass, a lying source never dropped. Each was a specific
//! instance of the same underlying question -- "can chaos ever
//! permanently wedge the scheduler" -- so this generalizes it into one
//! property instead of waiting to hand-write the next regression test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::PathBuf;

use dessplay::download::{DownloadAction, DownloadConfig, Downloads};
use dessplay_core::hash::{Ed2kBlockHash, ed2k_hash_bytes};
use dessplay_core::net::{Bitfield, CHUNK_SIZE, PeerId, PeerMessage, chunk_count, chunk_range};
use proptest::prelude::*;

const PEER_NAMES: &[&str] = &["a", "b", "c"];

fn peer(i: usize) -> PeerId {
    PeerId::new(PEER_NAMES[i % PEER_NAMES.len()])
}

/// One chaotic round: a peer misbehaves, or the source set churns.
#[derive(Debug, Clone)]
enum Chaos {
    LieAboutBlockHashes { peer: usize },
    SendCorruptChunk { peer: usize, index: u32 },
    Advertise { peer: usize },
    Churn { mask: u8 },
    Tick { millis: u64 },
}

fn chaos_strategy() -> impl Strategy<Value = Chaos> {
    prop_oneof![
        (0..PEER_NAMES.len()).prop_map(|peer| Chaos::LieAboutBlockHashes { peer }),
        (0..PEER_NAMES.len(), 0u32..40)
            .prop_map(|(peer, index)| Chaos::SendCorruptChunk { peer, index }),
        (0..PEER_NAMES.len()).prop_map(|peer| Chaos::Advertise { peer }),
        (0u8..8).prop_map(|mask| Chaos::Churn { mask }),
        (0u64..2000).prop_map(|millis| Chaos::Tick { millis }),
    ]
}

fn full_bitfield(n_chunks: u32) -> Bitfield {
    let mut bf = Bitfield::new(n_chunks);
    for i in 0..n_chunks {
        bf.set(i);
    }
    bf
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn chaos_prefix_never_prevents_eventual_completion(
        chaos in prop::collection::vec(chaos_strategy(), 0..30)
    ) {
        // 20 chunks: big enough that bulk (non-endgame) scheduling,
        // rarity, and the sequential window actually run; small enough
        // to hash fast across hundreds of proptest cases.
        let size_bytes: u64 = 20 * CHUNK_SIZE;
        let bytes: Vec<u8> = (0..size_bytes).map(|i| (i % 251) as u8).collect();
        let hash = ed2k_hash_bytes(&bytes);
        let n_chunks = chunk_count(size_bytes);
        let dir = tempfile::tempdir().unwrap();
        let path: PathBuf = dir.path().join("dl.bin");

        let config = DownloadConfig {
            pipeline_depth: 4,
            max_sources: 3,
            snub_timeout_millis: 200,
        };
        let mut downloads = Downloads::new(config);
        let mut now: u64 = 1000;
        let all: Vec<PeerId> = (0..PEER_NAMES.len()).map(peer).collect();
        downloads.start(hash.root, size_bytes, hash.root, path, all.clone(), 0, now);

        // Chaos prefix: every original source may lie, send corrupt
        // chunks, or vanish and reappear, in any order.
        for event in &chaos {
            now += 1;
            match event {
                Chaos::LieAboutBlockHashes { peer: p } => {
                    downloads.on_peer_message(
                        peer(*p),
                        PeerMessage::BlockHashes {
                            file: hash.root,
                            hashes: vec![Ed2kBlockHash([0xAB; 16]); hash.blocks.len()],
                        },
                        now,
                    );
                }
                Chaos::SendCorruptChunk { peer: p, index } => {
                    let index = index % n_chunks.max(1);
                    downloads.on_peer_message(
                        peer(*p),
                        PeerMessage::ChunkData {
                            file: hash.root,
                            index,
                            data: vec![0xFF; CHUNK_SIZE as usize],
                        },
                        now,
                    );
                }
                Chaos::Advertise { peer: p } => {
                    downloads.on_peer_message(
                        peer(*p),
                        PeerMessage::FileAvailability {
                            file: hash.root,
                            bitfield: full_bitfield(n_chunks),
                        },
                        now,
                    );
                }
                Chaos::Churn { mask } => {
                    let sources: Vec<PeerId> = (0..PEER_NAMES.len())
                        .filter(|i| mask & (1 << i) != 0)
                        .map(peer)
                        .collect();
                    downloads.set_sources(hash.root, sources, 0, now);
                }
                Chaos::Tick { millis } => {
                    now += millis;
                    downloads.tick(now);
                }
            }
        }

        // Honest epilogue: every original source is present again and
        // answers truthfully from here on. Past any snub timeout so
        // lingering bad state (silent sources, stale clocks) has a
        // chance to clear before we start counting.
        now += 10_000;
        let present: HashSet<String> = PEER_NAMES.iter().map(|s| s.to_string()).collect();
        let mut actions = downloads.set_sources(hash.root, all, 0, now);

        let mut completed = None;
        for _ in 0..100_000 {
            let mut next = Vec::new();
            for action in &actions {
                match action {
                    DownloadAction::Complete { path, .. } => completed = Some(path.clone()),
                    DownloadAction::Send { to, message } if present.contains(&to.to_string()) => {
                        let to = to.clone();
                        match message {
                            PeerMessage::BlockHashRequest { file } => {
                                let file = *file;
                                now += 1;
                                next.extend(downloads.on_peer_message(
                                    to.clone(),
                                    PeerMessage::BlockHashes {
                                        file,
                                        hashes: hash.blocks.clone(),
                                    },
                                    now,
                                ));
                                now += 1;
                                next.extend(downloads.on_peer_message(
                                    to,
                                    PeerMessage::FileAvailability {
                                        file,
                                        bitfield: full_bitfield(n_chunks),
                                    },
                                    now,
                                ));
                            }
                            PeerMessage::ChunkRequest { file, chunks } => {
                                let file = *file;
                                for &c in chunks {
                                    let r = chunk_range(c, size_bytes);
                                    now += 1;
                                    next.extend(downloads.on_peer_message(
                                        to.clone(),
                                        PeerMessage::ChunkData {
                                            file,
                                            index: c,
                                            data: bytes[r.start as usize..r.end as usize].to_vec(),
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
                if let DownloadAction::Complete { path, .. } = a {
                    completed = Some(path.clone());
                }
            }
            if completed.is_some() {
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

        let path = completed.expect(
            "must complete once every source is honest and present, regardless of the chaos prefix",
        );
        prop_assert_eq!(std::fs::read(&path).unwrap(), bytes, "assembled file must match exactly");
    }
}
