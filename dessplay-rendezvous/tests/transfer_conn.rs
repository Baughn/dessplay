//! Protocol v8: the transfer-connection split. The control connection
//! carries auth/state/presence; a second connection, bound by the
//! `AuthOk` token, carries relay streams. These tests pin the binding
//! rules and the split's core promise: transfer-link death never
//! touches presence.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::network::{NetworkCommand, NetworkEvent};
use dessplay::client::{ClientEvent, ClientHandle};
use dessplay_core::net::sim::EndpointId;
use dessplay_core::net::{
    Connector, PeerMessage, Presence, Role, ServerControl, Transport, TransportEvent, WireMessage,
};
use dessplay_core::types::{Ed2kHash, Epoch, UserId};
use dessplay_core::wire;

const BUDGET: Duration = Duration::from_secs(20);

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

/// Send `message` kim -> baughn until it lands (early sends are dropped
/// while relay streams register), within `budget`.
async fn relay_until_received(
    sender: &ClientHandle,
    receiver: &mut ClientHandle,
    message: PeerMessage,
    budget: Duration,
) {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        sender
            .network
            .send(NetworkCommand::SendPeer {
                to: UserId::new("baughn"),
                message: Box::new(message.clone()),
            })
            .await
            .unwrap();
        let got = tokio::time::timeout(Duration::from_millis(200), async {
            loop {
                match receiver.events.recv().await {
                    Some(ClientEvent::Network(NetworkEvent::Peer { message, .. })) => {
                        break Some(*message);
                    }
                    Some(_) => continue,
                    None => break None,
                }
            }
        })
        .await;
        if let Ok(Some(got)) = got {
            assert_eq!(got, message);
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "relayed message never arrived"
        );
    }
}

/// A transfer connection presenting a token the server never issued is
/// refused (closed), whether or not a session exists for the username.
#[tokio::test(start_paused = true)]
async fn transfer_connection_with_bad_token_is_refused() {
    let harness = Harness::new(910);
    // A real session for kim, so the "session exists, token wrong" path
    // is the one under test.
    let kim = harness.client("kim", 1);
    await_peer(&kim, "kim").await;

    let rogue = harness
        .net
        .connector(&EndpointId::new("rogue"), &harness.transfer_id)
        .connect()
        .await
        .expect("transfer listener reachable");
    let auth = WireMessage::Control(ServerControl::TransferAuth {
        username: UserId::new("kim"),
        token: 0xBAD_C0DE,
    });
    rogue
        .send_control(&wire::encode(&auth).unwrap())
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + BUDGET;
    loop {
        match tokio::time::timeout_at(deadline, rogue.recv()).await {
            Ok(Ok(TransportEvent::Closed { .. })) | Ok(Err(_)) => return,
            Ok(Ok(_)) => continue,
            Err(_) => panic!("bad-token transfer connection was not closed"),
        }
    }
}

/// A transfer connection for a username with no live session is refused.
#[tokio::test(start_paused = true)]
async fn transfer_connection_without_a_session_is_refused() {
    let harness = Harness::new(911);
    let ghost = harness
        .net
        .connector(&EndpointId::new("ghost"), &harness.transfer_id)
        .connect()
        .await
        .expect("transfer listener reachable");
    let auth = WireMessage::Control(ServerControl::TransferAuth {
        username: UserId::new("nobody"),
        token: 1,
    });
    ghost
        .send_control(&wire::encode(&auth).unwrap())
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + BUDGET;
    loop {
        match tokio::time::timeout_at(deadline, ghost.recv()).await {
            Ok(Ok(TransportEvent::Closed { .. })) | Ok(Err(_)) => return,
            Ok(Ok(_)) => continue,
            Err(_) => panic!("sessionless transfer connection was not closed"),
        }
    }
}

/// The split's core promise: killing a client's transfer connection
/// leaves its presence untouched (no Lost, no pause cascade), and the
/// self-healing link restores relay traffic on its own.
#[tokio::test(start_paused = true)]
async fn transfer_connection_death_degrades_transfers_not_presence() {
    let harness = Harness::new(912);
    let kim = harness.client("kim", 1);
    let mut baughn = harness.client("baughn", 2);
    await_peer(&kim, "baughn").await;
    await_peer(&baughn, "kim").await;

    // Relay works before the kill.
    let file = Ed2kHash([9; 16]);
    relay_until_received(
        &kim,
        &mut baughn,
        PeerMessage::BlockHashRequest { file },
        BUDGET,
    )
    .await;

    // Kill kim's transfer connection only. The control connection (and
    // with it presence) is untouched.
    harness
        .net
        .disconnect(&EndpointId::new("kim"), &harness.transfer_id);

    // Give any (wrong) presence fallout time to propagate, then assert
    // everyone is still Present on both sides.
    tokio::time::sleep(Duration::from_secs(5)).await;
    for handle in [&kim, &baughn] {
        let peers = handle.peers.borrow().clone();
        assert_eq!(peers.len(), 2, "both peers still listed");
        assert!(
            peers.iter().all(|p| p.presence == Presence::Present),
            "transfer-connection death must not touch presence: {peers:?}"
        );
    }

    // The transfer link redials itself (reconnect backoff), and relay
    // traffic resumes without any user-visible event.
    relay_until_received(
        &kim,
        &mut baughn,
        PeerMessage::ChunkRequest {
            file,
            chunks: vec![1, 2, 3],
        },
        BUDGET,
    )
    .await;
}

/// The server's per-transfer byte pump (protocol v9): a data stream
/// opened with an `OpenTransfer` header reaches the target as a fresh
/// stream headed `TransferFrom`, and bytes then flow both ways,
/// verbatim, through the pump. Exercised over full client stacks
/// elsewhere; this pins the server mechanics with raw peers.
#[tokio::test(start_paused = true)]
async fn a_data_stream_is_pumped_between_peers() {
    use dessplay_core::net::framing::{read_frame, write_frame};
    use dessplay_core::net::{BiStream, RelayEnvelope};

    let harness = Harness::new(914);
    // Two raw sessions: auth on the control connection, then bind a
    // transfer connection with the issued token — by hand, so the test
    // sees the pump's exact frames.
    let bind = |name: &'static str| {
        let harness = &harness;
        async move {
            let control = harness
                .net
                .connector(&EndpointId::new(name), &harness.server_id)
                .connect()
                .await
                .unwrap();
            let auth = WireMessage::Control(ServerControl::Auth {
                username: UserId::new(name),
                password: PASSWORD.into(),
                role: Role::Interactive,
                epoch: Epoch(0),
                protocol_version: dessplay_core::net::PROTOCOL_VERSION,
            });
            control
                .send_control(&wire::encode(&auth).unwrap())
                .await
                .unwrap();
            let token = loop {
                match control.recv().await.unwrap() {
                    TransportEvent::Control(bytes) => {
                        if let Ok(WireMessage::Control(ServerControl::AuthOk {
                            transfer_token,
                            ..
                        })) = wire::decode(&bytes)
                        {
                            break transfer_token;
                        }
                    }
                    TransportEvent::Closed { reason } => panic!("closed before AuthOk: {reason}"),
                    _ => continue,
                }
            };
            let transfer = harness
                .net
                .connector(&EndpointId::new(name), &harness.transfer_id)
                .connect()
                .await
                .unwrap();
            let bind = WireMessage::Control(ServerControl::TransferAuth {
                username: UserId::new(name),
                token,
            });
            transfer
                .send_control(&wire::encode(&bind).unwrap())
                .await
                .unwrap();
            (control, transfer)
        }
    };
    let (_kim_control, kim_transfer) = bind("kim").await;
    let (_baughn_control, baughn_transfer) = bind("baughn").await;
    // Let the TransferAuth frames land before opening streams.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let file = Ed2kHash([3; 16]);
    // kim (downloader) opens a data stream to baughn.
    let BiStream {
        send: mut kim_send,
        recv: mut kim_recv,
    } = kim_transfer.open_stream().await.unwrap();
    let header = RelayEnvelope::OpenTransfer {
        to: UserId::new("baughn"),
        file,
    };
    write_frame(&mut kim_send, &wire::encode(&header).unwrap())
        .await
        .unwrap();
    write_frame(&mut kim_send, b"request-bytes").await.unwrap();

    // baughn receives the pumped stream, headed TransferFrom.
    let deadline = tokio::time::Instant::now() + BUDGET;
    let BiStream {
        send: mut baughn_send,
        recv: mut baughn_recv,
    } = loop {
        match tokio::time::timeout_at(deadline, baughn_transfer.recv())
            .await
            .expect("pumped stream never arrived")
            .unwrap()
        {
            TransportEvent::IncomingStream(stream) => break stream,
            TransportEvent::Closed { reason } => panic!("transfer conn closed: {reason}"),
            _ => continue,
        }
    };
    let header_frame = read_frame(&mut baughn_recv).await.unwrap();
    assert_eq!(
        wire::decode::<RelayEnvelope>(&header_frame).unwrap(),
        RelayEnvelope::TransferFrom {
            from: UserId::new("kim"),
            file,
        }
    );
    // Downstream bytes arrived verbatim; upstream flows too.
    assert_eq!(
        read_frame(&mut baughn_recv).await.unwrap(),
        b"request-bytes"
    );
    write_frame(&mut baughn_send, b"data-bytes").await.unwrap();
    assert_eq!(read_frame(&mut kim_recv).await.unwrap(), b"data-bytes");
}

/// A reconnecting control connection invalidates the old session's
/// token: the transfer link that came with the *old* session cannot
/// serve the new one, but the new session's own link binds fine (the
/// client dials a fresh transfer connection with the fresh token).
#[tokio::test(start_paused = true)]
async fn reconnect_reissues_the_transfer_token() {
    let harness = Harness::new(913);

    // First session, raw: auth on the control connection, grab the token.
    let control = harness
        .net
        .connector(&EndpointId::new("kim"), &harness.server_id)
        .connect()
        .await
        .unwrap();
    let auth = WireMessage::Control(ServerControl::Auth {
        username: UserId::new("kim"),
        password: PASSWORD.into(),
        role: Role::Interactive,
        epoch: Epoch(0),
        protocol_version: dessplay_core::net::PROTOCOL_VERSION,
    });
    control
        .send_control(&wire::encode(&auth).unwrap())
        .await
        .unwrap();
    let first_token = loop {
        match control.recv().await.unwrap() {
            TransportEvent::Control(bytes) => {
                if let Ok(WireMessage::Control(ServerControl::AuthOk { transfer_token, .. })) =
                    wire::decode(&bytes)
                {
                    break transfer_token;
                }
            }
            TransportEvent::Closed { reason } => panic!("closed before AuthOk: {reason}"),
            _ => continue,
        }
    };

    // Second session for the same user (reconnect-supersede path).
    let control2 = harness
        .net
        .connector(&EndpointId::new("kim-laptop"), &harness.server_id)
        .connect()
        .await
        .unwrap();
    control2
        .send_control(&wire::encode(&auth).unwrap())
        .await
        .unwrap();
    let second_token = loop {
        match control2.recv().await.unwrap() {
            TransportEvent::Control(bytes) => {
                if let Ok(WireMessage::Control(ServerControl::AuthOk { transfer_token, .. })) =
                    wire::decode(&bytes)
                {
                    break transfer_token;
                }
            }
            TransportEvent::Closed { reason } => panic!("closed before AuthOk: {reason}"),
            _ => continue,
        }
    };
    assert_ne!(
        first_token, second_token,
        "a new session must issue a new token"
    );

    // The old token is dead: a transfer connection presenting it is
    // refused.
    let stale = harness
        .net
        .connector(&EndpointId::new("kim"), &harness.transfer_id)
        .connect()
        .await
        .unwrap();
    let stale_auth = WireMessage::Control(ServerControl::TransferAuth {
        username: UserId::new("kim"),
        token: first_token,
    });
    stale
        .send_control(&wire::encode(&stale_auth).unwrap())
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + BUDGET;
    loop {
        match tokio::time::timeout_at(deadline, stale.recv()).await {
            Ok(Ok(TransportEvent::Closed { .. })) | Ok(Err(_)) => break,
            Ok(Ok(_)) => continue,
            Err(_) => panic!("stale-token transfer connection was not closed"),
        }
    }
}
