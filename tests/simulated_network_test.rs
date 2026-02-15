mod common;

use common::simulated_network::{LinkConfig, SimulatedNetwork};
use dessplay::network::{ConnectionManager, PeerId};

#[tokio::test]
async fn datagram_send_recv_clean_network() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("alice".into()));
    let b = net.add_peer(PeerId("bob".into()));

    let msg = b"hello from alice";
    a.send_datagram(&PeerId("bob".into()), msg)
        .await
        .unwrap();

    let (from, data) = b.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, msg);
}

#[tokio::test]
async fn reliable_send_recv_clean_network() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("alice".into()));
    let b = net.add_peer(PeerId("bob".into()));

    let msg = b"reliable hello";
    a.send_reliable(&PeerId("bob".into()), msg).await.unwrap();

    let (from, data) = b.recv_reliable().await.unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, msg);
}

#[tokio::test]
async fn datagram_silently_dropped_on_partition() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("alice".into()));
    let _b = net.add_peer(PeerId("bob".into()));

    net.partition(&PeerId("alice".into()), &PeerId("bob".into()));

    // Datagram send should succeed (silently dropped)
    let result = a.send_datagram(&PeerId("bob".into()), b"dropped").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn reliable_fails_on_partition() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("alice".into()));
    let _b = net.add_peer(PeerId("bob".into()));

    net.partition(&PeerId("alice".into()), &PeerId("bob".into()));

    let result = a.send_reliable(&PeerId("bob".into()), b"fail").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn heal_restores_connectivity() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("alice".into()));
    let b = net.add_peer(PeerId("bob".into()));

    net.partition(&PeerId("alice".into()), &PeerId("bob".into()));
    net.heal(&PeerId("alice".into()), &PeerId("bob".into()));

    a.send_datagram(&PeerId("bob".into()), b"healed")
        .await
        .unwrap();

    let (from, data) = b.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, b"healed");
}

#[tokio::test]
async fn connected_peers_excludes_self_and_partitioned() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("alice".into()));
    let _b = net.add_peer(PeerId("bob".into()));
    let _c = net.add_peer(PeerId("charlie".into()));

    let peers = a.connected_peers();
    assert_eq!(peers.len(), 2);
    assert!(peers.contains(&PeerId("bob".into())));
    assert!(peers.contains(&PeerId("charlie".into())));

    net.partition(&PeerId("alice".into()), &PeerId("bob".into()));
    let peers = a.connected_peers();
    assert_eq!(peers.len(), 1);
    assert!(peers.contains(&PeerId("charlie".into())));
}

#[tokio::test(start_paused = true)]
async fn latency_simulation_with_paused_time() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("alice".into()));
    let b = net.add_peer(PeerId("bob".into()));

    net.set_link_symmetric(
        &PeerId("alice".into()),
        &PeerId("bob".into()),
        LinkConfig {
            latency_ms: 100,
            jitter_ms: 0,
            loss_rate: 0.0,
            reorder_rate: 0.0,
        },
    );

    let start = tokio::time::Instant::now();
    a.send_datagram(&PeerId("bob".into()), b"delayed")
        .await
        .unwrap();

    let (_from, _data) = b.recv_datagram().await.unwrap();
    let elapsed = start.elapsed();

    // With paused time, the 100ms delay should be simulated
    assert!(
        elapsed >= std::time::Duration::from_millis(95),
        "expected ~100ms delay, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn three_peer_bidirectional_communication() {
    let net = SimulatedNetwork::new(42);
    let a = net.add_peer(PeerId("a".into()));
    let b = net.add_peer(PeerId("b".into()));
    let c = net.add_peer(PeerId("c".into()));

    // A sends to B and C
    a.send_datagram(&PeerId("b".into()), b"to-b")
        .await
        .unwrap();
    a.send_datagram(&PeerId("c".into()), b"to-c")
        .await
        .unwrap();

    let (from_b, data_b) = b.recv_datagram().await.unwrap();
    assert_eq!(from_b, PeerId("a".into()));
    assert_eq!(data_b, b"to-b");

    let (from_c, data_c) = c.recv_datagram().await.unwrap();
    assert_eq!(from_c, PeerId("a".into()));
    assert_eq!(data_c, b"to-c");

    // B sends back to A
    b.send_datagram(&PeerId("a".into()), b"reply")
        .await
        .unwrap();
    let (from, data) = a.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("b".into()));
    assert_eq!(data, b"reply");
}
