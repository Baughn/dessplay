//! An arbitrary sequence of peer protocol events (honest and adversarial:
//! wrong-length bitfields, forged block hashes, out-of-range or corrupt
//! chunk data, churning source sets, clock jumps) through the download
//! scheduler must never panic.
//!
//! This targets `Downloads`, the chunk-scheduling state machine
//! (`dessplay::download`) -- the file's own regression-test comments
//! describe a run of prior wedge/double-assignment bugs found by hand
//! (stalled block-hash sources, a departed driving source, overlapping
//! bitfields assigned the same chunk in one pass, a source lying about
//! its block hashes), so it is exactly the kind of state machine that
//! benefits from adversarial-input fuzzing rather than only hand-picked
//! scenarios. Liveness ("does chaos ever wedge it permanently") is
//! covered separately by the `download_props` proptest, which can force
//! a clean epilogue and assert completion; this target's job is just
//! crash-safety against malformed peer input.

#![no_main]

use std::path::PathBuf;

use arbitrary::Arbitrary;
use dessplay::download::{DownloadConfig, Downloads};
use dessplay_core::hash::{Ed2kBlockHash, ed2k_hash_bytes};
use dessplay_core::net::{Bitfield, PeerId, PeerMessage, chunk_count, chunk_range};
use libfuzzer_sys::fuzz_target;

const PEERS: &[&str] = &["a", "b", "c"];

fn peer(i: u8) -> PeerId {
    PeerId::new(PEERS[(i as usize) % PEERS.len()])
}

/// One relayed peer message or scheduling event, honest or adversarial.
#[derive(Debug, Arbitrary)]
enum Event {
    /// A peer advertises its bitfield: the true full one, or a bogus
    /// wrong-length one (exercises `Bitfield::is_valid_for` rejection).
    Advertise { peer: u8, honest: bool },
    /// A peer answers a `BlockHashRequest`: the real hashes, or garbage
    /// that won't match the file root.
    SendBlockHashes { peer: u8, honest: bool },
    /// A peer delivers one chunk, at a possibly out-of-range index: the
    /// real bytes, or garbage of an arbitrary (possibly wrong) length.
    SendChunk {
        peer: u8,
        index: u16,
        honest: bool,
        garbage_len: u8,
    },
    /// Change which peers are considered present (join/leave/flakiness).
    SetSources { mask: u8 },
    /// Advance the shared clock (crosses snub/re-solicit timeouts).
    Tick { millis: u16 },
    /// Cancel the download (a local copy appeared through another
    /// channel), optionally restarting it — exercises cancel's cleanup
    /// and a re-start over the partially-written backing file.
    Cancel { restart: bool },
}

fuzz_target!(|events: Vec<Event>| {
    if events.is_empty() {
        return;
    }
    let Ok(dir) = tempfile::tempdir() else {
        return;
    };

    // A small backing file with real content and a real ed2k hash, so
    // "honest" events can supply genuinely valid data to verify against.
    let size_bytes: u64 = 3 * dessplay_core::net::CHUNK_SIZE;
    let bytes: Vec<u8> = (0..size_bytes).map(|i| (i % 251) as u8).collect();
    let hash = ed2k_hash_bytes(&bytes);
    let n_chunks = chunk_count(size_bytes);
    let path: PathBuf = dir.path().join("dl.bin");

    let config = DownloadConfig {
        pipeline_depth: 4,
        max_sources: 2,
        snub_timeout_millis: 500,
        urgent_age_millis: 250,
    };
    let mut downloads = Downloads::new(config);
    let mut now: u64 = 1000;
    let all_sources: Vec<PeerId> = PEERS.iter().map(|p| PeerId::new(*p)).collect();
    let _ = downloads.start(
        hash.root,
        size_bytes,
        hash.root,
        path.clone(),
        all_sources.clone(),
        0,
        now,
    );

    for event in events.into_iter().take(2000) {
        now += 1;
        match event {
            Event::Advertise { peer: p, honest } => {
                let bf = if honest {
                    let mut bf = Bitfield::new(n_chunks);
                    for i in 0..n_chunks {
                        bf.set(i);
                    }
                    bf
                } else {
                    // Deliberately wrong length: must be rejected, not panic.
                    Bitfield::new(n_chunks.wrapping_add(7).max(1))
                };
                let _ = downloads.on_peer_message(
                    peer(p),
                    PeerMessage::FileAvailability {
                        file: hash.root,
                        bitfield: bf,
                    },
                    now,
                );
            }
            Event::SendBlockHashes { peer: p, honest } => {
                let hashes = if honest {
                    hash.blocks.clone()
                } else {
                    vec![Ed2kBlockHash([0xAB; 16]); hash.blocks.len().max(1)]
                };
                let _ = downloads.on_peer_message(
                    peer(p),
                    PeerMessage::BlockHashes {
                        file: hash.root,
                        hashes,
                    },
                    now,
                );
            }
            Event::SendChunk {
                peer: p,
                index,
                honest,
                garbage_len,
            } => {
                // Deliberately allow indices at and beyond `n_chunks`, to
                // exercise the out-of-range path.
                let index = (index as u32) % (n_chunks as u32 * 2).max(1);
                let data = if honest && index < n_chunks {
                    let r = chunk_range(index, size_bytes);
                    bytes[r.start as usize..r.end as usize].to_vec()
                } else {
                    vec![0xAA; garbage_len as usize]
                };
                let _ = downloads.on_peer_message(
                    peer(p),
                    PeerMessage::ChunkData {
                        file: hash.root,
                        index,
                        data,
                    },
                    now,
                );
            }
            Event::SetSources { mask } => {
                let sources: Vec<PeerId> = PEERS
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, name)| PeerId::new(*name))
                    .collect();
                let _ = downloads.set_sources(hash.root, sources, 0, now);
            }
            Event::Tick { millis } => {
                now = now.saturating_add(millis as u64);
                let _ = downloads.tick(now);
            }
            Event::Cancel { restart } => {
                let _ = downloads.cancel(&hash.root);
                if restart {
                    let _ = downloads.start(
                        hash.root,
                        size_bytes,
                        hash.root,
                        path.clone(),
                        all_sources.clone(),
                        0,
                        now,
                    );
                }
            }
        }
    }
});
