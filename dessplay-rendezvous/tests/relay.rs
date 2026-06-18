//! Phase 9B-1: file-transfer relay. Two clients exchange a
//! `PeerMessage` through the server over the dedicated relay stream —
//! the bytes travel downloader -> server -> uploader, never directly.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::network::{NetworkCommand, NetworkEvent};
use dessplay::client::{ClientEvent, ClientHandle};
use dessplay_core::net::{Bitfield, PeerMessage};
use dessplay_core::types::{Ed2kHash, UserId};

const BUDGET: Duration = Duration::from_secs(20);

/// Wait until `name` appears in `handle`'s peer list (so it has authed
/// and, just after, opened its relay stream).
async fn await_peer(handle: &ClientHandle, name: &str) {
    let deadline = tokio::time::Instant::now() + BUDGET;
    while tokio::time::Instant::now() < deadline {
        if handle
            .peers
            .borrow()
            .iter()
            .any(|p| p.username == UserId::new(name))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{name} never appeared in the peer list");
}

/// Drain `handle.events` for the next relayed peer message, if one
/// arrives within `budget`.
async fn try_recv_peer(
    handle: &mut ClientHandle,
    budget: Duration,
) -> Option<(UserId, PeerMessage)> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        match tokio::time::timeout(Duration::from_millis(50), handle.events.recv()).await {
            Ok(Some(ClientEvent::Network(NetworkEvent::Peer { from, message }))) => {
                return Some((from, *message));
            }
            Ok(Some(_)) => continue,
            Ok(None) => return None,
            Err(_) if tokio::time::Instant::now() >= deadline => return None,
            Err(_) => continue,
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_peer_message_is_relayed_through_the_server() {
    let harness = Harness::new(901);
    let kim = harness.client("kim", 1);
    let mut baughn = harness.client("baughn", 2);

    // Both authenticated (and thus relay streams opening).
    await_peer(&kim, "baughn").await;
    await_peer(&baughn, "kim").await;

    let file = Ed2kHash([7; 16]);
    let mut bitfield = Bitfield::new(10);
    bitfield.set(0);
    bitfield.set(3);
    let message = PeerMessage::FileAvailability {
        file,
        bitfield: bitfield.clone(),
    };

    // kim -> baughn, retried: a send before baughn's relay stream is
    // registered on the server is silently dropped, so keep trying until
    // it lands (the relay stream opens just after AuthOk).
    let deadline = tokio::time::Instant::now() + BUDGET;
    let received = loop {
        kim.network
            .send(NetworkCommand::SendPeer {
                to: UserId::new("baughn"),
                message: Box::new(message.clone()),
            })
            .await
            .unwrap();
        if let Some(got) = try_recv_peer(&mut baughn, Duration::from_millis(200)).await {
            break got;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "baughn never received the relayed message"
        );
    };

    let (from, got) = received;
    assert_eq!(from, UserId::new("kim"), "envelope carries the sender");
    assert_eq!(
        got, message,
        "the peer message survives the relay round trip"
    );
}

#[tokio::test(start_paused = true)]
async fn messages_to_an_absent_peer_are_dropped_not_fatal() {
    // Sending to a peer who isn't connected is a silent no-op (the
    // server drops envelopes to non-present peers); the sender's
    // connection stays healthy and later real traffic still works.
    let harness = Harness::new(902);
    let kim = harness.client("kim", 1);
    let mut baughn = harness.client("baughn", 2);
    await_peer(&kim, "baughn").await;
    await_peer(&baughn, "kim").await;

    let file = Ed2kHash([1; 16]);
    // To a ghost: dropped.
    kim.network
        .send(NetworkCommand::SendPeer {
            to: UserId::new("ghost"),
            message: Box::new(PeerMessage::BlockHashRequest { file }),
        })
        .await
        .unwrap();

    // kim's connection is still fine: a real message to baughn lands.
    let message = PeerMessage::ChunkRequest {
        file,
        chunks: vec![1, 2, 3],
    };
    let deadline = tokio::time::Instant::now() + BUDGET;
    let received = loop {
        kim.network
            .send(NetworkCommand::SendPeer {
                to: UserId::new("baughn"),
                message: Box::new(message.clone()),
            })
            .await
            .unwrap();
        if let Some(got) = try_recv_peer(&mut baughn, Duration::from_millis(200)).await {
            break got;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "message never arrived"
        );
    };
    assert_eq!(received.1, message);
}
