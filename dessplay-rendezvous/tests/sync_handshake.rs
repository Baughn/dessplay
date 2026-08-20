//! The connect handshake's server half (protocol v13): after `AuthOk`
//! the client reports what it holds via `SyncStatus { epoch, state_hash }`,
//! and the server decides the initial sync from BOTH — `StateMerge` iff
//! epoch AND hash match its own, `StateSnapshot` otherwise. A bare
//! epoch match must never buy a merge: after a DB restore the epoch
//! counter can collide while the states differ (the 2026-08 tsugumi
//! incident), and merging then re-pollutes the restored state.
//!
//! Also pinned here: the divergence heal (`RequestMerge`) is answered
//! with a `StateSnapshot` — curative, since a snapshot removes
//! client-local garbage where a union cannot.
//!
//! Raw-wire client, so each server decision is observed directly
//! (template: op_rebroadcast.rs).

use std::sync::Arc;
use std::time::Duration;

use dessplay_core::net::sim::{EndpointId, SimNetwork, SimTransport};
use dessplay_core::net::{
    Connector, PROTOCOL_VERSION, Role, ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::types::{Epoch, UserId};
use dessplay_core::{CrdtState, wire};
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

/// A fresh sim network with the real (storageless) server listening on
/// it: epoch 1, empty state.
fn setup(seed: u64) -> (SimNetwork, EndpointId) {
    let net = SimNetwork::new(seed);
    let server_id = EndpointId::new("server");
    let listener = net.listener(&server_id);
    let transfer_listener = net.listener(&EndpointId::new("server-transfer"));
    tokio::spawn(server::run(
        listener,
        transfer_listener,
        ServerConfig::new(PASSWORD),
        sim_clock(),
        None,
    ));
    (net, server_id)
}

/// Connect a raw transport, authenticate as `name`, wait for `AuthOk`,
/// and drain until idle — so anything the server volunteers around auth
/// (peer lists) is out of the way before the test acts.
async fn connect_authed(net: &SimNetwork, server: &EndpointId, name: &str) -> SimTransport {
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
    loop {
        match conn.recv().await.expect("recv during auth") {
            TransportEvent::Control(bytes) => match wire::decode::<WireMessage>(&bytes).unwrap() {
                WireMessage::Control(ServerControl::AuthOk { .. }) => break,
                WireMessage::Control(ServerControl::AuthFailed) => panic!("auth failed"),
                _ => continue,
            },
            _ => continue,
        }
    }
    // Let anything already in flight land, then discard it.
    while let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(500), conn.recv()).await {}
    conn
}

/// Await the next state-sync reply (`StateSnapshot` or `StateMerge`),
/// skipping everything else; panics if none arrives within the budget.
async fn next_sync_reply(conn: &SimTransport, what: &str) -> ServerControl {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match conn.recv().await.expect("recv") {
                TransportEvent::Control(bytes) => {
                    match wire::decode::<WireMessage>(&bytes).unwrap() {
                        WireMessage::Control(msg @ ServerControl::StateSnapshot(_))
                        | WireMessage::Control(msg @ ServerControl::StateMerge(_)) => break msg,
                        _ => continue,
                    }
                }
                _ => continue,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no state-sync reply to {what}"))
}

fn sync_status(epoch: Epoch, state_hash: [u8; 32]) -> Vec<u8> {
    wire::encode(&WireMessage::Control(ServerControl::SyncStatus {
        epoch,
        state_hash,
    }))
    .unwrap()
}

/// Epoch AND hash match: the cheap path — a merge (which is a no-op to
/// apply on an identical replica) rather than a full snapshot.
#[tokio::test(start_paused = true)]
async fn equal_epoch_and_matching_hash_get_a_merge() {
    let (net, server) = setup(0x31);
    let conn = connect_authed(&net, &server, "kim").await;

    // A fresh storageless server holds an empty state at epoch 1.
    let matching = sync_status(Epoch(1), CrdtState::new().view_hash());
    conn.send_control(&matching).await.expect("send SyncStatus");

    let reply = next_sync_reply(&conn, "a matching SyncStatus").await;
    let ServerControl::StateMerge(snapshot) = reply else {
        panic!("epoch+hash match must be answered with StateMerge, got {reply:?}");
    };
    assert_eq!(snapshot.epoch, Epoch(1));
}

/// Epoch matches but the hash does not — the restore-collision shape.
/// Must be a snapshot: a merge would union the client's divergent state
/// straight back into the server.
#[tokio::test(start_paused = true)]
async fn equal_epoch_with_a_wrong_hash_gets_a_snapshot() {
    let (net, server) = setup(0x32);
    let conn = connect_authed(&net, &server, "kim").await;

    let mismatched = sync_status(Epoch(1), [0xAB; 32]);
    conn.send_control(&mismatched)
        .await
        .expect("send SyncStatus");

    let reply = next_sync_reply(&conn, "a hash-mismatched SyncStatus").await;
    let ServerControl::StateSnapshot(snapshot) = reply else {
        panic!(
            "an epoch match with a WRONG hash must be answered with StateSnapshot, got {reply:?}"
        );
    };
    assert_eq!(snapshot.epoch, Epoch(1));
}

/// The divergence heal: `RequestMerge` is answered with a `StateSnapshot`
/// (curative — the requester's view hash already mismatched twice, so
/// its replica holds something the server does not; a snapshot removes
/// it, a union re-spreads it).
#[tokio::test(start_paused = true)]
async fn request_merge_heal_is_answered_with_a_snapshot() {
    let (net, server) = setup(0x33);
    let conn = connect_authed(&net, &server, "kim").await;

    let heal = wire::encode(&WireMessage::Control(ServerControl::RequestMerge)).unwrap();
    conn.send_control(&heal).await.expect("send RequestMerge");

    let reply = next_sync_reply(&conn, "RequestMerge").await;
    let ServerControl::StateSnapshot(snapshot) = reply else {
        panic!("the divergence heal must be answered with StateSnapshot, got {reply:?}");
    };
    assert_eq!(snapshot.epoch, Epoch(1));
}
