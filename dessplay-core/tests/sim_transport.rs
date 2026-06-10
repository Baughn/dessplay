//! SimulatedTransport behavior: ordering, latency, loss, partitions,
//! bandwidth, jitter-reordering, and close semantics. All under paused
//! time — simulated seconds are free.

use std::time::Duration;

use dessplay_core::net::sim::{EndpointId, LinkConfig, SimNetwork};
use dessplay_core::net::{Connector, Listener, Transport, TransportError, TransportEvent};

fn ids() -> (EndpointId, EndpointId) {
    (EndpointId::new("client"), EndpointId::new("server"))
}

async fn connected(net: &SimNetwork) -> (impl Transport, impl Transport) {
    let (client_id, server_id) = ids();
    let listener = net.listener(&server_id);
    let connector = net.connector(&client_id, &server_id);
    let client = connector.connect().await.expect("connect");
    let (server, _addr) = listener.accept().await.expect("accept");
    (client, server)
}

/// recv() with a zero-length budget: succeeds only if already deliverable.
async fn try_recv<T: Transport>(t: &T) -> Option<TransportEvent> {
    tokio::time::timeout(Duration::ZERO, t.recv())
        .await
        .ok()?
        .ok()
}

#[tokio::test(start_paused = true)]
async fn control_frames_arrive_in_order() {
    let net = SimNetwork::new(1);
    let (client, server) = connected(&net).await;

    for i in 0..10u8 {
        client.send_control(&[i]).await.unwrap();
    }
    for i in 0..10u8 {
        match server.recv().await.unwrap() {
            TransportEvent::Control(frame) => assert_eq!(frame, vec![i]),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    // And the reverse direction works.
    server.send_control(b"pong").await.unwrap();
    assert!(matches!(
        client.recv().await.unwrap(),
        TransportEvent::Control(f) if f == b"pong"
    ));
}

#[tokio::test(start_paused = true)]
async fn latency_delays_delivery() {
    let net = SimNetwork::new(1);
    let (client_id, server_id) = ids();
    net.set_link(
        &client_id,
        &server_id,
        LinkConfig {
            latency: Duration::from_millis(500),
            ..LinkConfig::default()
        },
    );
    let (client, server) = connected(&net).await;

    client.send_control(b"slow").await.unwrap();
    assert!(try_recv(&server).await.is_none(), "arrived too early");
    tokio::time::sleep(Duration::from_millis(501)).await;
    assert!(try_recv(&server).await.is_some(), "never arrived");
}

#[tokio::test(start_paused = true)]
async fn datagram_loss_and_reliability_contrast() {
    let net = SimNetwork::new(7);
    let (client_id, server_id) = ids();
    net.set_link(
        &client_id,
        &server_id,
        LinkConfig {
            datagram_loss: 1.0,
            ..LinkConfig::default()
        },
    );
    let (client, server) = connected(&net).await;

    // Total datagram loss: nothing arrives.
    for _ in 0..5 {
        client.send_datagram(b"gone").await.unwrap();
    }
    // Control frames are unaffected.
    client.send_control(b"kept").await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(matches!(
        server.recv().await.unwrap(),
        TransportEvent::Control(f) if f == b"kept"
    ));
    assert!(try_recv(&server).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn partition_holds_control_drops_datagrams() {
    let net = SimNetwork::new(3);
    let (client_id, server_id) = ids();
    let (client, server) = connected(&net).await;

    net.set_partitioned(&client_id, &server_id, true);
    client.send_control(b"held").await.unwrap();
    client.send_datagram(b"dropped").await.unwrap();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(try_recv(&server).await.is_none(), "partition leaked");

    net.set_partitioned(&client_id, &server_id, false);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert!(matches!(
        server.recv().await.unwrap(),
        TransportEvent::Control(f) if f == b"held"
    ));
    // The datagram is gone forever.
    assert!(try_recv(&server).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn bandwidth_paces_delivery() {
    let net = SimNetwork::new(1);
    let (client_id, server_id) = ids();
    net.set_link(
        &client_id,
        &server_id,
        LinkConfig {
            bandwidth: Some(1_000), // 1000 bytes/sec
            ..LinkConfig::default()
        },
    );
    let (client, server) = connected(&net).await;

    // Two 500-byte frames: ~0.5s and ~1.0s serialization points.
    client.send_control(&[1u8; 500]).await.unwrap();
    client.send_control(&[2u8; 500]).await.unwrap();

    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(try_recv(&server).await.is_some(), "first frame late");
    assert!(try_recv(&server).await.is_none(), "second frame early");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(try_recv(&server).await.is_some(), "second frame missing");
}

#[tokio::test(start_paused = true)]
async fn jitter_reorders_datagrams() {
    let net = SimNetwork::new(42);
    let (client_id, server_id) = ids();
    net.set_link(
        &client_id,
        &server_id,
        LinkConfig {
            datagram_jitter: Duration::from_millis(100),
            ..LinkConfig::default()
        },
    );
    let (client, server) = connected(&net).await;

    let count = 20u8;
    for i in 0..count {
        client.send_datagram(&[i]).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut order = Vec::new();
    while let Some(TransportEvent::Datagram(frame)) = try_recv(&server).await {
        order.push(frame[0]);
    }
    assert_eq!(order.len(), count as usize, "datagrams lost without loss");
    let sorted: Vec<u8> = (0..count).collect();
    assert_ne!(
        order, sorted,
        "jitter produced no reordering (seed-dependent)"
    );
}

#[tokio::test(start_paused = true)]
async fn datagram_size_rule() {
    let net = SimNetwork::new(1);
    let (client, _server) = connected(&net).await;
    assert_eq!(client.max_datagram_size(), Some(1200));
    let oversized = vec![0u8; 1300];
    assert!(matches!(
        client.send_datagram(&oversized).await,
        Err(TransportError::DatagramTooLarge {
            len: 1300,
            max: 1200
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn close_notifies_peer_then_errors() {
    let net = SimNetwork::new(1);
    let (client, server) = connected(&net).await;

    client.close("bye").await;
    assert!(matches!(
        server.recv().await.unwrap(),
        TransportEvent::Closed { reason } if reason == "bye"
    ));
    assert!(client.send_control(b"x").await.is_err());
    // The closer's own recv unblocks too.
    let mut saw_close = false;
    for _ in 0..2 {
        match client.recv().await {
            Ok(TransportEvent::Closed { .. }) | Err(_) => {
                saw_close = true;
                break;
            }
            Ok(_) => continue,
        }
    }
    assert!(saw_close);
}

#[tokio::test(start_paused = true)]
async fn streams_open_across_the_link() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let net = SimNetwork::new(1);
    let (client, server) = connected(&net).await;

    let mut local = client.open_stream().await.unwrap();
    let TransportEvent::IncomingStream(mut remote) = server.recv().await.unwrap() else {
        panic!("expected incoming stream");
    };

    local.send.write_all(b"chunk").await.unwrap();
    local.send.flush().await.unwrap();
    let mut buf = [0u8; 5];
    remote.recv.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"chunk");
}

#[tokio::test(start_paused = true)]
async fn dropping_a_transport_is_silent() {
    let net = SimNetwork::new(1);
    let (client, server) = connected(&net).await;

    drop(client);
    tokio::time::sleep(Duration::from_secs(120)).await;
    // The peer hears nothing — that's what presence timeouts are for.
    assert!(try_recv(&server).await.is_none());
}
