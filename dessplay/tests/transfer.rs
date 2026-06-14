//! Phase 9B end-to-end transfer over a modelled relay: real `FileActor`s
//! (seeder + leecher) wired output-to-input through a pump that plays the
//! server's relay role. Asserts the leecher assembles the file
//! byte-for-byte, and reports **transfer efficiency** — goodput
//! (file bytes / bytes transmitted) and retransmit % — the metric the
//! design calls for.
//!
//! The relay's job is pure message forwarding, which the pump models
//! exactly; the sim transport's streams bypass its link model anyway, so
//! this is both more deterministic and a truer measure of *protocol*
//! efficiency (duplicate sends, refetches) than routing bytes through a
//! sim socket would be.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use dessplay::actors::file::{FileCommand, FileConfig, FileOutput, run};
use dessplay::config::CacheRetention;
use dessplay::download::DownloadConfig;
use dessplay::storage::Storage;
use dessplay_core::hash::{ED2K_BLOCK_SIZE, ed2k_hash_bytes};
use dessplay_core::net::{PeerId, PeerMessage};
use dessplay_core::types::Ed2kHash;
use tokio::sync::mpsc;

/// A spawned file actor with its channels and a kept-alive cache dir.
struct Actor {
    name: String,
    commands: mpsc::Sender<FileCommand>,
    outputs: mpsc::Receiver<FileOutput>,
    _cache: tempfile::TempDir,
    _media: tempfile::TempDir,
}

fn clock() -> dessplay::actors::network::Clock {
    std::sync::Arc::new(|| 1_700_000_000_000)
}

fn spawn_actor(name: &str, media_files: &[(&str, &[u8])]) -> Actor {
    let media = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    for (filename, bytes) in media_files {
        std::fs::write(media.path().join(filename), bytes).unwrap();
    }
    let (cmd_tx, cmd_rx) = mpsc::channel(256);
    let (out_tx, out_rx) = mpsc::channel(1024);
    tokio::spawn(run(
        FileConfig {
            storage: Storage::open_in_memory().unwrap(),
            media_roots: vec![media.path().to_path_buf()],
            retention: CacheRetention::default(),
            cache_dir: cache.path().to_path_buf(),
            clock: clock(),
            download: DownloadConfig::default(),
            upload_limit: None,
            scan_interval: None,
        },
        cmd_rx,
        out_tx,
    ));
    Actor {
        name: name.to_string(),
        commands: cmd_tx,
        outputs: out_rx,
        _cache: cache,
        _media: media,
    }
}

/// Outcome of a pumped transfer.
struct Outcome {
    /// Where the leecher assembled the file.
    completed_path: Option<PathBuf>,
    /// Total ChunkData bytes relayed (useful + wasted).
    chunk_bytes: u64,
}

/// Pump the actors' relay messages between each other until the named
/// leecher reports the file complete (or a timeout). Counts ChunkData
/// bytes for the efficiency metric.
async fn pump_until_complete(actors: &mut [Actor], leecher: &str, budget: Duration) -> Outcome {
    let senders: HashMap<String, mpsc::Sender<FileCommand>> = actors
        .iter()
        .map(|a| (a.name.clone(), a.commands.clone()))
        .collect();
    let deadline = tokio::time::Instant::now() + budget;
    let mut chunk_bytes = 0u64;
    let mut completed_path = None;

    while tokio::time::Instant::now() < deadline {
        // Poll every actor's output channel without blocking forever.
        let mut idle = true;
        for actor in actors.iter_mut() {
            let from = actor.name.clone();
            let event =
                match tokio::time::timeout(Duration::from_millis(20), actor.outputs.recv()).await {
                    Ok(Some(event)) => event,
                    Ok(None) => continue,
                    Err(_) => continue, // nothing right now
                };
            idle = false;
            match event {
                FileOutput::SendPeer { to, message } => {
                    if let PeerMessage::ChunkData { data, .. } = &*message {
                        chunk_bytes += data.len() as u64;
                    }
                    if let Some(target) = senders.get(&to.to_string()) {
                        let _ = target
                            .send(FileCommand::PeerMessage {
                                from: PeerId::new(&from),
                                message,
                            })
                            .await;
                    }
                }
                FileOutput::DownloadComplete { path, .. } if from == leecher => {
                    completed_path = Some(path);
                }
                _ => {}
            }
            if completed_path.is_some() {
                break;
            }
        }
        if completed_path.is_some() {
            break;
        }
        if idle {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    Outcome {
        completed_path,
        chunk_bytes,
    }
}

/// Make the seeder resolve (hash) a local file so it can serve both the
/// block hashes and chunks. Returns once it reports Verified.
async fn make_seeder_ready(seeder: &mut Actor, filename: &str, hash: Ed2kHash) {
    seeder
        .commands
        .send(FileCommand::Resolve {
            file: hash,
            filename: filename.to_string(),
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), seeder.outputs.recv()).await {
            Ok(Some(FileOutput::Resolved { file, resolution })) if file == hash => {
                assert!(
                    matches!(resolution, dessplay::actors::file::Resolution::Verified(_)),
                    "seeder should verify its own file"
                );
                return;
            }
            Ok(Some(_)) => continue,
            _ => continue,
        }
    }
    panic!("seeder never became ready");
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(37) % 251) as u8).collect()
}

#[tokio::test]
async fn single_seed_transfer_completes_with_full_goodput() {
    let bytes = data(2 * ED2K_BLOCK_SIZE as usize + 123_456); // 3 blocks
    let hash = ed2k_hash_bytes(&bytes);
    let filename = "movie.mkv";

    let mut seeder = spawn_actor("seed", &[(filename, &bytes)]);
    let leecher = spawn_actor("leech", &[]);
    make_seeder_ready(&mut seeder, filename, hash.root).await;

    // The leecher learns "seed" has the file and starts downloading.
    leecher
        .commands
        .send(FileCommand::StartDownload {
            file: hash.root,
            size_bytes: hash.size_bytes,
            sources: vec![PeerId::new("seed")],
            play_chunk: 0,
        })
        .await
        .unwrap();

    let mut actors = vec![seeder, leecher];
    let outcome = pump_until_complete(&mut actors, "leech", Duration::from_secs(30)).await;

    let path = outcome.completed_path.expect("leecher should complete");
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "assembled file matches");

    // Efficiency: every byte transmitted was useful and sent once, so
    // goodput is ~100% and retransmits ~0 (a single clean source).
    let goodput_bps = (hash.size_bytes * 10_000) / outcome.chunk_bytes;
    let retransmit_bps = ((outcome.chunk_bytes - hash.size_bytes) * 10_000) / outcome.chunk_bytes;
    println!(
        "single-seed: {} file bytes, {} transmitted — goodput {}.{:02}%, retransmit {}.{:02}%",
        hash.size_bytes,
        outcome.chunk_bytes,
        goodput_bps / 100,
        goodput_bps % 100,
        retransmit_bps / 100,
        retransmit_bps % 100,
    );
    assert!(goodput_bps >= 9_900, "goodput should be ~100%: {goodput_bps} bps");
    assert!(retransmit_bps <= 100, "retransmit should be ~0%: {retransmit_bps} bps");
}

#[tokio::test]
async fn two_seed_transfer_completes_and_stays_efficient() {
    // Two complete sources: rarest-first/source-cap spread the load;
    // endgame may briefly double-request the tail, so allow a small
    // retransmit budget but still expect high goodput.
    let bytes = data(3 * ED2K_BLOCK_SIZE as usize + 50_000); // 4 blocks
    let hash = ed2k_hash_bytes(&bytes);
    let filename = "show.mkv";

    let mut seed_a = spawn_actor("seedA", &[(filename, &bytes)]);
    let mut seed_b = spawn_actor("seedB", &[(filename, &bytes)]);
    let leecher = spawn_actor("leech", &[]);
    make_seeder_ready(&mut seed_a, filename, hash.root).await;
    make_seeder_ready(&mut seed_b, filename, hash.root).await;

    leecher
        .commands
        .send(FileCommand::StartDownload {
            file: hash.root,
            size_bytes: hash.size_bytes,
            sources: vec![PeerId::new("seedA"), PeerId::new("seedB")],
            play_chunk: 0,
        })
        .await
        .unwrap();

    let mut actors = vec![seed_a, seed_b, leecher];
    let outcome = pump_until_complete(&mut actors, "leech", Duration::from_secs(30)).await;

    let path = outcome.completed_path.expect("leecher should complete");
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "assembled file matches");

    let goodput_bps = (hash.size_bytes * 10_000) / outcome.chunk_bytes;
    println!(
        "two-seed: {} file bytes, {} transmitted — goodput {}.{:02}%",
        hash.size_bytes,
        outcome.chunk_bytes,
        goodput_bps / 100,
        goodput_bps % 100,
    );
    // Even with two sources and endgame, almost nothing is wasted.
    assert!(goodput_bps >= 9_500, "goodput should stay high: {goodput_bps} bps");
}
