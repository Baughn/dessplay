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
            scan_transfer_quiet: dessplay::actors::file::SCAN_TRANSFER_QUIET_DEFAULT,
            torrent: None,
            nyaa: None,
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
    /// Total data-stream bytes pumped uploader -> downloader (framed
    /// ChunkData; useful + wasted, like the server's pump would carry).
    chunk_bytes: u64,
}

/// A connected pair of in-process streams (what the sim transport does
/// for its `BiStream`s).
fn bistream_pair() -> (dessplay_core::net::BiStream, dessplay_core::net::BiStream) {
    let (a, b) = tokio::io::duplex(256 * 1024);
    let (a_read, a_write) = tokio::io::split(a);
    let (b_read, b_write) = tokio::io::split(b);
    (
        dessplay_core::net::BiStream {
            send: Box::new(a_write),
            recv: Box::new(a_read),
        },
        dessplay_core::net::BiStream {
            send: Box::new(b_write),
            recv: Box::new(b_read),
        },
    )
}

/// Model the server's per-transfer byte pump: two stream pairs joined by
/// bounded copy tasks (so backpressure propagates end to end, exactly
/// like the real pump), counting the uploader->downloader bytes.
fn pumped_stream_pair(
    counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> (dessplay_core::net::BiStream, dessplay_core::net::BiStream) {
    let (down_local, down_far) = bistream_pair();
    let (up_local, up_far) = bistream_pair();
    let dessplay_core::net::BiStream {
        send: mut up_send,
        recv: mut up_recv,
    } = up_far;
    let dessplay_core::net::BiStream {
        send: mut down_send,
        recv: mut down_recv,
    } = down_far;
    // Requests: downloader -> uploader.
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut down_recv, &mut up_send).await;
    });
    // Data: uploader -> downloader, counted.
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 16 * 1024];
        loop {
            match up_recv.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    counter.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                    if down_send.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    (down_local, up_local)
}

/// Pump the actors' relay messages between each other — and stand in for
/// the server's data-stream pump on `OpenTransfer` — until the named
/// leecher reports the file complete (or a timeout).
async fn pump_until_complete(actors: &mut [Actor], leecher: &str, budget: Duration) -> Outcome {
    let senders: HashMap<String, mpsc::Sender<FileCommand>> = actors
        .iter()
        .map(|a| (a.name.clone(), a.commands.clone()))
        .collect();
    let deadline = tokio::time::Instant::now() + budget;
    let chunk_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
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
                    if let Some(target) = senders.get(&to.to_string()) {
                        let _ = target
                            .send(FileCommand::PeerMessage {
                                from: PeerId::new(&from),
                                message,
                            })
                            .await;
                    }
                }
                FileOutput::OpenTransfer { to, file } => {
                    // The server's role: join the two ends with a byte
                    // pump. The downloader gets its stream back
                    // (outbound), the uploader an incoming serve stream.
                    let Some(target) = senders.get(&to.to_string()) else {
                        continue; // absent peer: stream just never opens
                    };
                    let (down, up) = pumped_stream_pair(std::sync::Arc::clone(&chunk_bytes));
                    let _ = senders[&from]
                        .send(FileCommand::TransferStream {
                            peer: to.clone(),
                            file,
                            outbound: true,
                            stream: down,
                        })
                        .await;
                    let _ = target
                        .send(FileCommand::TransferStream {
                            peer: PeerId::new(&from),
                            file,
                            outbound: false,
                            stream: up,
                        })
                        .await;
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
        chunk_bytes: chunk_bytes.load(std::sync::atomic::Ordering::Relaxed),
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

/// Regression (2026-07-05 review): when a local copy appears through
/// another channel mid-transfer, the download is cancelled and its
/// partial deleted — but the session's `resolved` map lags the file
/// actor's, so a snapshot processed in that window re-emits
/// `StartDownload`. That stale re-emit must be a no-op: without the
/// local-files guard it re-created the just-deleted partial and
/// re-downloaded the entire file from the relay.
#[tokio::test]
async fn a_cancelled_redundant_download_is_not_resurrected_by_a_stale_start() {
    let bytes = data(64 * 1024);
    let hash = ed2k_hash_bytes(&bytes);
    let filename = "ep1.mkv";
    let mut leecher = spawn_actor("leech", &[]);
    let partial = leecher._cache.path().join(hash.root.to_string());

    // A download starts and solicits its source (no replies needed —
    // the point is what the *leecher* sends, not a full transfer).
    let start = || FileCommand::StartDownload {
        file: hash.root,
        size_bytes: hash.size_bytes,
        sources: vec![PeerId::new("seed")],
        play_chunk: 0,
    };
    leecher.commands.send(start()).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "download never solicited its source"
        );
        match tokio::time::timeout(Duration::from_millis(50), leecher.outputs.recv()).await {
            Ok(Some(FileOutput::SendPeer { message, .. }))
                if matches!(&*message, PeerMessage::BlockHashRequest { .. }) =>
            {
                break;
            }
            _ => continue,
        }
    }

    // The local copy lands through another channel; resolving it
    // verifies, cancels the download, and deletes the partial.
    std::fs::write(leecher._media.path().join(filename), &bytes).unwrap();
    leecher
        .commands
        .send(FileCommand::Resolve {
            file: hash.root,
            filename: filename.to_string(),
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "local copy never resolved"
        );
        match tokio::time::timeout(Duration::from_millis(50), leecher.outputs.recv()).await {
            Ok(Some(FileOutput::Resolved { file, resolution })) if file == hash.root => {
                assert!(
                    matches!(resolution, dessplay::actors::file::Resolution::Verified(_)),
                    "the landed copy should verify"
                );
                break;
            }
            _ => continue,
        }
    }
    // Drain any cancel-time traffic (e.g. drop-requests to sources).
    while tokio::time::timeout(Duration::from_millis(100), leecher.outputs.recv())
        .await
        .ok()
        .flatten()
        .is_some()
    {}
    assert!(!partial.exists(), "cancel should have deleted the partial");

    // The stale session re-emit: must not re-create the partial or
    // solicit anyone.
    leecher.commands.send(start()).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(FileOutput::SendPeer { message, .. })) =
            tokio::time::timeout(Duration::from_millis(50), leecher.outputs.recv()).await
        {
            panic!("stale StartDownload resurrected the download: sent {message:?}");
        }
    }
    assert!(
        !partial.exists(),
        "stale StartDownload re-created the deleted partial"
    );
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
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "assembled file matches"
    );

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
    assert!(
        goodput_bps >= 9_900,
        "goodput should be ~100%: {goodput_bps} bps"
    );
    assert!(
        retransmit_bps <= 100,
        "retransmit should be ~0%: {retransmit_bps} bps"
    );
}

#[tokio::test]
async fn two_seed_transfer_completes_and_stays_efficient() {
    // Two complete sources, both genuinely solicited and used (before the
    // 2026-06-26 review fix, only one was ever solicited, so this silently
    // ran single-source). bulk-mode planning must not double-assign a
    // chunk, so the *only* wasted bytes are the endgame tail: the last
    // <= pipeline_depth chunks are requested from both seeds and the loser
    // is Cancelled. We bound the waste by exactly that tail — a regression
    // to bulk-mode duplication would blow past it (goodput would collapse
    // toward 50%). On a real (large) file this tail is a negligible
    // fraction; it only looks large here because the test file is small.
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
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "assembled file matches"
    );

    let goodput_bps = (hash.size_bytes * 10_000) / outcome.chunk_bytes;
    let wasted = outcome.chunk_bytes - hash.size_bytes;
    // The endgame tail: at most pipeline_depth chunks re-requested from the
    // second seed.
    let endgame_tail =
        u64::from(DownloadConfig::default().pipeline_depth) * dessplay_core::net::CHUNK_SIZE;
    println!(
        "two-seed: {} file bytes, {} transmitted — goodput {}.{:02}%, wasted {wasted} (tail bound {endgame_tail})",
        hash.size_bytes,
        outcome.chunk_bytes,
        goodput_bps / 100,
        goodput_bps % 100,
    );
    // Bulk mode must not duplicate: all waste is the bounded endgame tail.
    assert!(
        wasted <= endgame_tail,
        "waste {wasted} exceeds the endgame tail {endgame_tail} — bulk mode is duplicating chunks"
    );
}

/// The head-of-line fix (protocol v9): a downloader that requests
/// everything and then never reads must not stall serving to anyone
/// else. Pre-v9 all recipients shared one relay stream and one
/// actor-loop serve queue, so exactly this — one saturated or stalled
/// peer — starved every other transfer. Now each serve runs on its own
/// stream/task and blocks only itself.
#[tokio::test]
async fn a_stalled_downloader_does_not_starve_another() {
    use dessplay_core::net::framing::write_frame;

    let bytes = data(ED2K_BLOCK_SIZE as usize + 77_000); // 2 blocks
    let hash = ed2k_hash_bytes(&bytes);
    let filename = "night.mkv";

    let mut seeder = spawn_actor("seed", &[(filename, &bytes)]);
    let leecher = spawn_actor("leech", &[]);
    make_seeder_ready(&mut seeder, filename, hash.root).await;

    // The stalled peer: hand the seeder a serve stream directly, request
    // every chunk on it, and never read a byte of the reply. The serve
    // task fills the stream buffer and blocks — alone.
    let (mut far, near) = {
        let (far, near) = {
            let (a, b) = tokio::io::duplex(64 * 1024);
            let (a_read, a_write) = tokio::io::split(a);
            let (b_read, b_write) = tokio::io::split(b);
            (
                dessplay_core::net::BiStream {
                    send: Box::new(a_write) as Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
                    recv: Box::new(a_read) as Box<dyn tokio::io::AsyncRead + Send + Unpin>,
                },
                dessplay_core::net::BiStream {
                    send: Box::new(b_write) as Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
                    recv: Box::new(b_read) as Box<dyn tokio::io::AsyncRead + Send + Unpin>,
                },
            )
        };
        (far, near)
    };
    seeder
        .commands
        .send(FileCommand::TransferStream {
            peer: PeerId::new("slow"),
            file: hash.root,
            outbound: false,
            stream: near,
        })
        .await
        .unwrap();
    let all_chunks: Vec<u32> = (0..dessplay_core::net::chunk_count(hash.size_bytes)).collect();
    let request = dessplay_core::wire::encode(&PeerMessage::ChunkRequest {
        file: hash.root,
        chunks: all_chunks,
    })
    .unwrap();
    write_frame(&mut far.send, &request).await.unwrap();
    // Keep `far` alive (dropping it would close the stream and free the
    // serve task) but never read: the definition of a stalled peer.

    // Meanwhile a healthy leecher downloads the same file.
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
    let path = outcome
        .completed_path
        .expect("the healthy leecher must complete despite the stalled peer");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "assembled file matches"
    );
    drop(far);
}
