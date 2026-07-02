//! Regression: the server must not rebroadcast the *same* eager op twice.
//!
//! Every ordinary (non-position) op is sent **eager** — a reliable control
//! copy *and* a datagram copy of the same op (see `send_eager`). The server
//! therefore receives two copies and applies both, but it must fan the op
//! out to the other peers only **once**: the copy that actually changes the
//! resolved state. Pre-fix the reliable path hardcoded `applied = true` and
//! the datagram fast path's order-free arm (`apply_if_orderly`) returned
//! `true` even for an already-present element, so both copies of an
//! order-free op (registers, the lookup/ack GSets, chat) were rebroadcast —
//! ~2x op fan-out (sync-state.md, Operation Broadcast: "the server
//! deduplicates, applies, and broadcasts").
//!
//! This end-to-end test exercises the order-free leak with a chat op (a
//! GList insert, no playback side effects). The map-op arm has its own
//! asymmetry — the reliable path used to return `true` unconditionally, so a
//! *datagram-first* map op rebroadcast twice — covered by the deterministic
//! unit test `eager_map_op_rebroadcasts_once_on_either_transport_first` in
//! `dessplay-core/src/state.rs`.

use std::sync::Arc;
use std::time::Duration;

use dessplay_core::net::sim::{EndpointId, SimNetwork, SimTransport};
use dessplay_core::net::{
    Connector, PROTOCOL_VERSION, Role, ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::types::{ChatMessage, Epoch, SharedTimestamp, UserId};
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

/// Connect a raw transport, authenticate as `name` (epoch 0, so the server
/// replies with a snapshot), and drain that snapshot. Returns the
/// connection and the epoch the server advertised.
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
        protocol_version: PROTOCOL_VERSION,
    });
    conn.send_control(&wire::encode(&auth).unwrap())
        .await
        .expect("send auth");
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

/// Drain every event currently deliverable to `conn`, stopping once it goes
/// idle for `window` (paused time, so this is cheap).
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

/// A chat op authored in a throwaway empty state, so its GList identifier is
/// deterministic and applies cleanly (and idempotently) on the server.
fn chat_op(name: &str, text: &str) -> CrdtOp {
    let mut local = CrdtState::new();
    local.append_chat(ChatMessage {
        sender: UserId::new(name),
        text: text.into(),
        timestamp: SharedTimestamp(1_700_000_000_001),
    })
}

fn is_chat(op: &CrdtOp) -> bool {
    matches!(op, CrdtOp::Chat(_))
}

/// The bug: an eager order-free op (sent as a reliable control copy *and* a
/// datagram copy of the same op) must be rebroadcast to other peers exactly
/// once — only the copy that changes state. Pre-fix both copies rebroadcast,
/// so baughn sees the op twice over the reliable control stream (`control ==
/// 2`); the fix suppresses the second, no-op copy (`control == 1`).
#[tokio::test(start_paused = true)]
async fn an_eager_order_free_op_is_rebroadcast_once() {
    let (net, server) = setup(0x10);
    let (kim, epoch) = connect_authed(&net, &server, "kim").await;
    let (baughn, _) = connect_authed(&net, &server, "baughn").await;

    // kim sends one chat op eager: the reliable control copy AND the
    // datagram copy of the *same* op, exactly as `send_eager` puts it on
    // the wire.
    let op = chat_op("kim", "hello");
    let frame = wire::encode(&WireMessage::Control(ServerControl::StateOp { epoch, op })).unwrap();
    kim.send_control(&frame).await.expect("send chat reliable");
    kim.send_datagram(&frame).await.expect("send chat datagram");

    // Let the server receive, apply, and relay both copies before we look.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = drain(&baughn, Duration::from_millis(200)).await;
    let (control, datagram) = relayed_by_channel(&events, is_chat);

    assert_eq!(
        control, 1,
        "an eager order-free op must rebroadcast exactly once over the \
         reliable control stream (the no-op duplicate is suppressed); saw {control}"
    );
    assert!(
        datagram <= 1,
        "...and at most once over the datagram channel; saw {datagram}"
    );
}
