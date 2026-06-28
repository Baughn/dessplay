//! Regression: the server must **mirror the inbound transport** when it
//! relays playback-position ops.
//!
//! Clients send `PlaybackPosition` ops datagram-only at the 100ms cadence
//! (with a 1s reliable catch-up tick) precisely to keep stale positions
//! off the reliable control stream — relaying every position reliably is
//! "exactly the head-of-line blocking we are avoiding" (network-design.md,
//! "Exception -- playback position"; sync-state.md, Playback Position).
//!
//! The relay path must honor that: a position that arrived on the
//! datagram fast path is relayed datagram-only, while a position that
//! arrived reliably (the 1s tick) and every other op type relay eagerly
//! (reliable + an eager datagram copy). These tests observe *which*
//! channel each relayed op leaves the server on by driving raw
//! [`SimTransport`] peers and inspecting the [`TransportEvent`] variant.

use std::sync::Arc;
use std::time::Duration;

use dessplay_core::net::sim::{EndpointId, SimNetwork, SimTransport};
use dessplay_core::net::{Connector, Role, ServerControl, Transport, TransportEvent, WireMessage};
use dessplay_core::types::{ActorId, Ed2kHash, Epoch, PlaybackPosition, SharedTimestamp, UserId};
use dessplay_core::{CrdtOp, CrdtState, wire};
use dessplay_rendezvous::server::{self, ServerConfig};

const PASSWORD: &str = "hunter2";

/// A clock following paused tokio time from a fixed origin.
fn sim_clock() -> Arc<dyn Fn() -> u64 + Send + Sync> {
    let origin = tokio::time::Instant::now();
    Arc::new(move || {
        let elapsed = tokio::time::Instant::now().duration_since(origin);
        1_700_000_000_000_u64 + elapsed.as_millis() as u64
    })
}

/// A fresh sim network with the real server listening on it.
fn setup(seed: u64) -> (SimNetwork, EndpointId) {
    let net = SimNetwork::new(seed);
    let server_id = EndpointId::new("server");
    let listener = net.listener(&server_id);
    tokio::spawn(server::run(
        listener,
        ServerConfig::new(PASSWORD),
        sim_clock(),
        None,
    ));
    (net, server_id)
}

/// Connect a raw transport, authenticate as `name` (epoch 0, so the
/// server replies with a snapshot), and drain that snapshot. Returns the
/// connection and the epoch the server advertised — what later ops must
/// be tagged with.
async fn connect_authed(
    net: &SimNetwork,
    server: &EndpointId,
    name: &str,
) -> (SimTransport, Epoch) {
    let conn = net
        .connector(&EndpointId::new(name), server)
        .connect()
        .await
        .expect("connect");
    let auth = WireMessage::Control(ServerControl::Auth {
        username: UserId::new(name),
        password: PASSWORD.into(),
        role: Role::Interactive,
        epoch: Epoch(0),
    });
    conn.send_control(&wire::encode(&auth).unwrap())
        .await
        .expect("send auth");
    // Drain replies until the initial snapshot/merge arrives.
    let epoch = loop {
        match conn.recv().await.expect("recv during auth") {
            TransportEvent::Control(bytes) => match wire::decode::<WireMessage>(&bytes).unwrap() {
                WireMessage::Control(ServerControl::StateSnapshot(s)) => break s.epoch,
                WireMessage::Control(ServerControl::StateMerge(s)) => break s.epoch,
                _ => continue,
            },
            _ => continue,
        }
    };
    (conn, epoch)
}

/// Drain every event currently deliverable to `conn`, stopping once the
/// connection goes idle for `window` (paused time, so this is cheap).
async fn drain(conn: &SimTransport, window: Duration) -> Vec<TransportEvent> {
    let mut out = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(window, conn.recv()).await {
        out.push(event);
    }
    out
}

/// Count relayed `StateOp`s whose op matches `pred`, split by the channel
/// they arrived on: `(reliable_control, datagram)`.
fn relayed_by_channel(events: &[TransportEvent], pred: impl Fn(&CrdtOp) -> bool) -> (usize, usize) {
    let mut control = 0;
    let mut datagram = 0;
    for event in events {
        let (bytes, is_datagram) = match event {
            TransportEvent::Control(bytes) => (bytes, false),
            TransportEvent::Datagram(bytes) => (bytes, true),
            _ => continue,
        };
        if let Ok(WireMessage::Control(ServerControl::StateOp { op, .. })) =
            wire::decode::<WireMessage>(bytes)
            && pred(&op)
        {
            if is_datagram {
                datagram += 1;
            } else {
                control += 1;
            }
        }
    }
    (control, datagram)
}

/// A `PlaybackPosition` op for `name` at `position_millis`, counter 1 in a
/// throwaway state (so it applies in-sequence on the server's fresh map).
fn position_op(name: &str, position_millis: u64) -> CrdtOp {
    let mut local = CrdtState::new();
    let ts = SharedTimestamp(1_700_000_000_001);
    local.set_playback_position(
        ActorId::session(name, 1),
        ts,
        UserId::new(name),
        PlaybackPosition {
            position_millis,
            timestamp: ts,
            file: Ed2kHash([1; 16]),
        },
    )
}

fn is_position(op: &CrdtOp) -> bool {
    matches!(op, CrdtOp::PlaybackPosition(_))
}

fn is_now_playing(op: &CrdtOp) -> bool {
    matches!(op, CrdtOp::NowPlaying(_))
}

/// The bug: a position that arrived on the 100ms datagram fast path must
/// be relayed **datagram-only** — never re-fanned-out over the reliable
/// control stream. Pre-fix the server relays it eagerly (reliable + a
/// datagram copy), so the reliable count is 1 and this fails.
#[tokio::test(start_paused = true)]
async fn a_datagram_position_is_relayed_datagram_only() {
    let (net, server) = setup(0x1);
    let (kim, epoch) = connect_authed(&net, &server, "kim").await;
    let (baughn, _) = connect_authed(&net, &server, "baughn").await;

    // kim reports a position on the datagram fast path.
    let op = position_op("kim", 1234);
    let frame = wire::encode(&WireMessage::Control(ServerControl::StateOp { epoch, op })).unwrap();
    kim.send_datagram(&frame).await.expect("send position");

    // Let the server receive, apply, and relay before we look.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = drain(&baughn, Duration::from_millis(200)).await;
    let (control, datagram) = relayed_by_channel(&events, is_position);

    assert!(
        datagram >= 1,
        "baughn must receive the relayed position on the datagram channel"
    );
    assert_eq!(
        control, 0,
        "a datagram-received position must NOT be relayed over the reliable \
         control stream (head-of-line blocking the design forbids); saw {control}"
    );
}

/// The mirror's other half: a position that arrived on the **reliable**
/// path (the 1s catch-up tick, sent eager by the client) relays reliably,
/// so laggards on a lossy link still get a baseline. Holds before and
/// after the fix.
#[tokio::test(start_paused = true)]
async fn a_reliable_position_relays_reliably() {
    let (net, server) = setup(0x2);
    let (kim, epoch) = connect_authed(&net, &server, "kim").await;
    let (baughn, _) = connect_authed(&net, &server, "baughn").await;

    let op = position_op("kim", 5678);
    let frame = wire::encode(&WireMessage::Control(ServerControl::StateOp { epoch, op })).unwrap();
    kim.send_control(&frame).await.expect("send position");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = drain(&baughn, Duration::from_millis(200)).await;
    let (control, _datagram) = relayed_by_channel(&events, is_position);

    assert!(
        control >= 1,
        "a reliably-received position (the 1s tick) must relay over the \
         reliable control stream"
    );
}

/// No regression for ordinary ops: a non-position op keeps its eager
/// relay, which includes the reliable control stream.
#[tokio::test(start_paused = true)]
async fn a_non_position_op_still_relays_reliably() {
    let (net, server) = setup(0x3);
    let (kim, epoch) = connect_authed(&net, &server, "kim").await;
    let (baughn, _) = connect_authed(&net, &server, "baughn").await;

    let mut local = CrdtState::new();
    let op = local.set_now_playing(
        ActorId::session("kim", 1),
        SharedTimestamp(1_700_000_000_001),
        Some(Ed2kHash([7; 16])),
    );
    let frame = wire::encode(&WireMessage::Control(ServerControl::StateOp { epoch, op })).unwrap();
    kim.send_control(&frame).await.expect("send now-playing");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = drain(&baughn, Duration::from_millis(200)).await;
    let (control, _datagram) = relayed_by_channel(&events, is_now_playing);

    assert!(
        control >= 1,
        "a non-position op must keep relaying over the reliable control stream"
    );
}
