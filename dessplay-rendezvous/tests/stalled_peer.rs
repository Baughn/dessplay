//! Regression (2026-10-28): one connection that stops reading must not
//! stall the server for everyone else.
//!
//! The incident: a third-party client stopped reading its control stream
//! (its QUIC window filled during a burst of reconnect-merge
//! rebroadcasts). The server sent every broadcast to each peer in turn
//! and awaited each write, so the first broadcast that reached the
//! stalled peer never returned — and with it the serve loop of whichever
//! client had triggered it, which then stopped answering that client's
//! time-sync probes ("probes lost"), the StateHash ticker, and every new
//! login (stuck in the peer-list broadcast after "superseding"). Nothing
//! recovered until the server was restarted.
//!
//! The sim's control-stream window ([`SimNetwork::set_control_window`])
//! models the honest trigger: a peer that authenticates and then simply
//! never calls `recv()` again. The relay stream (small peer messages on
//! the transfer connection) is the same shape and gets the same test.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::network::{NetworkCommand, NetworkEvent};
use dessplay::actors::sync::Mutation;
use dessplay::client::{ClientEvent, ClientHandle};
use dessplay_core::net::framing::write_frame;
use dessplay_core::net::sim::{EndpointId, SimTransport};
use dessplay_core::net::{
    Bitfield, Connector, PROTOCOL_VERSION, PeerMessage, Presence, RelayEnvelope, Role,
    ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::types::{Ed2kHash, Epoch, UserId};
use dessplay_core::{CrdtState, wire};
use dessplay_rendezvous::server::ServerConfig;

const BUDGET: Duration = Duration::from_secs(30);

/// A misbehaving client: authenticates, completes the connect handshake
/// up to `AuthOk`, and from then on never reads its control connection
/// again. Returns the control connection (held, so it stays alive) and
/// the session's transfer token.
async fn rogue_control(harness: &Harness, name: &str) -> (SimTransport, u64) {
    let conn = harness
        .net
        .connector(&EndpointId::new(name), &harness.server_id)
        .connect()
        .await
        .expect("connect");
    let auth = WireMessage::Control(ServerControl::Auth {
        username: UserId::new(name),
        password: PASSWORD.into(),
        role: Role::Seeder,
        epoch: Epoch(0),
        protocol_version: PROTOCOL_VERSION,
    });
    conn.send_control(&wire::encode(&auth).unwrap())
        .await
        .unwrap();
    let token = loop {
        if let TransportEvent::Control(bytes) = conn.recv().await.expect("recv AuthOk")
            && let Ok(WireMessage::Control(ServerControl::AuthOk { transfer_token, .. })) =
                wire::decode::<WireMessage>(&bytes)
        {
            break transfer_token;
        }
    };
    // Ask for the initial sync like a real client, then go deaf.
    let status = WireMessage::Control(ServerControl::SyncStatus {
        epoch: Epoch(0),
        state_hash: CrdtState::new().view_hash(),
    });
    conn.send_control(&wire::encode(&status).unwrap())
        .await
        .unwrap();
    (conn, token)
}

/// `count` chat lines of ~`len` bytes each from `author`.
async fn chatter(author: &ClientHandle, tag: &str, count: usize, len: usize) {
    for i in 0..count {
        let text = format!("{tag}{i:04}:{}", "x".repeat(len));
        mutate(author, Mutation::Chat { text }).await;
    }
}

fn chat_count(snapshot: &ClientSnapshot, tag: &str) -> usize {
    snapshot
        .view
        .chat
        .iter()
        .filter(|m| m.text.starts_with(tag))
        .count()
}

/// A peer that never reads its control stream must not freeze anyone
/// else: ops keep flowing between the healthy clients, and a fresh
/// login still completes and syncs.
#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_control_does_not_stall_the_others() {
    init_test_logging();
    let harness = Harness::new(4401);
    // Small enough that the chatter below overflows it many times over.
    harness.net.set_control_window(16 * 1024);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    eventually(&[&kim, &baughn], BUDGET, |s| {
        s.iter()
            .all(|c| c.peer("kim").is_some() && c.peer("baughn").is_some())
    })
    .await;
    let (_rogue, _token) = rogue_control(&harness, "rogue").await;

    // ~100 KiB of broadcasts: far past the rogue's window.
    chatter(&kim, "hello", 50, 2000).await;
    eventually(&[&baughn], BUDGET, |s| chat_count(&s[0], "hello") == 50).await;

    // A login after the rogue's window filled still authenticates and
    // gets the whole state.
    let nero = harness.client("nero", 3);
    eventually(&[&nero], BUDGET, |s| chat_count(&s[0], "hello") == 50).await;
}

/// The stalled peer's backlog is bounded: once its outbound queue passes
/// the limit, the server drops the connection (it would resync with a
/// snapshot on reconnect), and the others see it go Lost.
#[tokio::test(start_paused = true)]
async fn a_peer_that_never_drains_is_disconnected_once_its_backlog_overflows() {
    init_test_logging();
    let mut config = ServerConfig::new(PASSWORD);
    config.peer_outbox_limit = 64 * 1024;
    let harness = Harness::with_config(4402, config);
    harness.net.set_control_window(16 * 1024);
    let kim = harness.client("kim", 1);
    let (_rogue, _token) = rogue_control(&harness, "rogue").await;
    eventually(&[&kim], BUDGET, |s| {
        s[0].peer("rogue")
            .is_some_and(|p| p.presence == Presence::Present)
    })
    .await;

    // ~200 KiB: past the window plus the 64 KiB outbox limit.
    chatter(&kim, "flood", 100, 2000).await;
    eventually(&[&kim], BUDGET, |s| {
        s[0].peer("rogue")
            .is_some_and(|p| p.presence != Presence::Present)
    })
    .await;
}

/// Wait until `name` appears in `handle`'s peer list.
async fn await_peer(handle: &ClientHandle, name: &str) {
    eventually(&[handle], BUDGET, |s| s[0].peer(name).is_some()).await;
}

/// Drain `handle.events` until a relayed message from `from` matching
/// `pred` arrives, or `budget` runs out.
async fn await_relayed(
    handle: &mut ClientHandle,
    from: &str,
    budget: Duration,
    mut pred: impl FnMut(&PeerMessage) -> bool,
) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), handle.events.recv()).await {
            Ok(Some(ClientEvent::Network(NetworkEvent::Peer { from: f, message })))
                if f == UserId::new(from) && pred(&message) =>
            {
                return true;
            }
            Ok(None) => return false,
            _ => continue,
        }
    }
    false
}

/// The relay stream sibling: a peer whose relay stream is never read
/// must not stall a sender's relayed messages to everyone else.
#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_its_relay_stream_does_not_stall_relays_to_others() {
    init_test_logging();
    let harness = Harness::new(4403);
    let kim = harness.client("kim", 1);
    let mut baughn = harness.client("baughn", 2);
    await_peer(&kim, "baughn").await;
    await_peer(&baughn, "kim").await;

    // The rogue binds a transfer connection and opens its relay stream,
    // then never reads either.
    let (_rogue, token) = rogue_control(&harness, "rogue").await;
    let transfer = harness
        .net
        .connector(&EndpointId::new("rogue"), &harness.transfer_id)
        .connect()
        .await
        .expect("transfer connect");
    let bind = WireMessage::Control(ServerControl::TransferAuth {
        username: UserId::new("rogue"),
        token,
    });
    transfer
        .send_control(&wire::encode(&bind).unwrap())
        .await
        .unwrap();
    let mut relay = transfer.open_stream().await.expect("relay stream");
    write_frame(
        &mut relay.send,
        &wire::encode(&RelayEnvelope::Hello).unwrap(),
    )
    .await
    .unwrap();
    await_peer(&kim, "rogue").await;
    // Let the server register the relay stream.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // ~1 MiB toward the rogue: well past the sim stream's 256 KiB buffer.
    let file = Ed2kHash([9; 16]);
    for _ in 0..80 {
        kim.network
            .send(NetworkCommand::SendPeer {
                to: UserId::new("rogue"),
                message: Box::new(PeerMessage::FileAvailability {
                    file,
                    bitfield: Bitfield::new(100_000),
                }),
            })
            .await
            .unwrap();
    }
    // Then something for baughn, behind the rogue's backlog in kim's
    // relay stream.
    let mut marker = Bitfield::new(10);
    marker.set(4);
    kim.network
        .send(NetworkCommand::SendPeer {
            to: UserId::new("baughn"),
            message: Box::new(PeerMessage::FileAvailability {
                file,
                bitfield: marker.clone(),
            }),
        })
        .await
        .unwrap();
    assert!(
        await_relayed(&mut baughn, "kim", BUDGET, |m| matches!(
            m,
            PeerMessage::FileAvailability { bitfield, .. } if *bitfield == marker
        ))
        .await,
        "kim's relay to baughn stalled behind the rogue's unread relay stream"
    );
}
