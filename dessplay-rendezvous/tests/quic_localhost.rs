//! Real-QUIC integration: actual quinn endpoints over localhost UDP,
//! real TLS with TOFU, real time. Small in number by design — logic
//! lives in the simulated tests; this proves the production transport
//! stack works end to end.

use std::sync::Arc;
use std::time::Duration;

use dessplay::actors::network::{self, NetworkCommand, NetworkConfig, NetworkEvent};
use dessplay_core::net::quic::{QuicConnector, QuicListener};
use dessplay_core::net::tofu::{fingerprint, load_or_generate_cert};
use dessplay_core::net::{Connector, Role};
use dessplay_core::types::UserId;
use dessplay_rendezvous::server::{self, ServerConfig, system_clock};
use std::sync::atomic::AtomicU64;
use tokio::sync::mpsc;

const PASSWORD: &str = "hunter2";

async fn expect_event<T>(
    events: &mut mpsc::Receiver<NetworkEvent>,
    budget: Duration,
    mut pred: impl FnMut(&NetworkEvent) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let event = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("event budget exhausted")
            .expect("event channel closed");
        if let Some(out) = pred(&event) {
            return out;
        }
    }
}

#[tokio::test]
async fn two_clients_connect_over_real_quic() {
    let cert_dir = tempfile::tempdir().unwrap();
    let (cert, key) = load_or_generate_cert(cert_dir.path()).unwrap();
    let expected_fp = fingerprint(cert.as_ref()).to_vec();

    let listener = QuicListener::bind("127.0.0.1:0".parse().unwrap(), cert, key).unwrap();
    let server_addr = listener.local_addr().unwrap();
    tokio::spawn(server::run(
        listener,
        ServerConfig::new(PASSWORD),
        system_clock(),
        None,
    ));

    let mut clients = Vec::new();
    let mut connectors = Vec::new();
    for name in ["kim", "baughn"] {
        // First use: no pin.
        let connector = Arc::new(QuicConnector::new(server_addr, "dessplay", None).unwrap());
        connectors.push(Arc::clone(&connector));
        let (_cmd_tx, cmd_rx) = mpsc::channel::<NetworkCommand>(8);
        let (event_tx, event_rx) = mpsc::channel(64);
        tokio::spawn(network::run(
            connector,
            NetworkConfig::new(
                UserId::new(name),
                PASSWORD.into(),
                Role::Interactive,
                Arc::new(AtomicU64::new(0)),
                Arc::new(|| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0)
                }),
            ),
            cmd_rx,
            event_tx,
        ));
        clients.push((event_rx, _cmd_tx));
    }

    // Accumulate events into per-client state and wait for the goal
    // condition (connected + both peers visible + clock synced) — events
    // arrive interleaved, so phased waiting would eat them.
    for (events, _) in &mut clients {
        let mut connected = false;
        let mut saw_both_peers = false;
        let mut offset: Option<i64> = None;
        expect_event(events, Duration::from_secs(10), |e| {
            match e {
                NetworkEvent::Connected { .. } => connected = true,
                NetworkEvent::PeerList(peers) if peers.len() == 2 => saw_both_peers = true,
                NetworkEvent::ClockSync { offset_millis } => offset = Some(*offset_millis),
                _ => {}
            }
            (connected && saw_both_peers && offset.is_some()).then_some(())
        })
        .await;
        // Same machine, same clock: the offset must be tiny.
        let offset = offset.unwrap_or(i64::MAX);
        assert!(offset.abs() < 100, "loopback clock offset {offset}ms");
    }

    // TOFU observed the real certificate.
    for connector in &connectors {
        assert_eq!(
            connector.observed_fingerprint().as_deref(),
            Some(expected_fp.as_slice())
        );
    }
}

/// A wrong password must surface as AuthFailed — a clean, terminal
/// exit — not as a generic connection loss followed by infinite
/// retries. Real QUIC only: closing right after sending AuthFailed
/// discards the unflushed frame, which the simulated transport is too
/// polite to reproduce.
#[tokio::test]
async fn wrong_password_fails_cleanly_over_real_quic() {
    let cert_dir = tempfile::tempdir().unwrap();
    let (cert, key) = load_or_generate_cert(cert_dir.path()).unwrap();
    let listener = QuicListener::bind("127.0.0.1:0".parse().unwrap(), cert, key).unwrap();
    let server_addr = listener.local_addr().unwrap();
    tokio::spawn(server::run(
        listener,
        ServerConfig::new(PASSWORD),
        system_clock(),
        None,
    ));

    let connector = Arc::new(QuicConnector::new(server_addr, "dessplay", None).unwrap());
    let (_cmd_tx, cmd_rx) = mpsc::channel::<NetworkCommand>(8);
    let (event_tx, mut events) = mpsc::channel(64);
    tokio::spawn(network::run(
        connector,
        NetworkConfig::new(
            UserId::new("kim"),
            "WRONG".into(),
            Role::Interactive,
            Arc::new(AtomicU64::new(0)),
            Arc::new(|| 0),
        ),
        cmd_rx,
        event_tx,
    ));

    // The first terminal-ish event must be AuthFailed; a Disconnected
    // means the rejection got eaten by the close and the client would
    // retry forever.
    expect_event(&mut events, Duration::from_secs(10), |e| match e {
        NetworkEvent::AuthFailed => Some(()),
        NetworkEvent::Disconnected { reason } => {
            panic!("rejection arrived as a generic disconnect: {reason}")
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn wrong_pinned_fingerprint_refuses_to_connect() {
    let cert_dir = tempfile::tempdir().unwrap();
    let (cert, key) = load_or_generate_cert(cert_dir.path()).unwrap();
    let listener = QuicListener::bind("127.0.0.1:0".parse().unwrap(), cert, key).unwrap();
    let server_addr = listener.local_addr().unwrap();
    tokio::spawn(server::run(
        listener,
        ServerConfig::new(PASSWORD),
        system_clock(),
        None,
    ));

    let connector = QuicConnector::new(server_addr, "dessplay", Some(vec![0xAA; 32])).unwrap();
    let result = connector.connect().await;
    assert!(result.is_err(), "connected despite a fingerprint mismatch");
    assert!(connector.observed_fingerprint().is_none());
}
