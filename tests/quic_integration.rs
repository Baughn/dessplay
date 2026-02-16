use std::time::Duration;

use dessplay::network::{ConnectionError, ConnectionEvent, ConnectionManager, PeerId};
use dessplay::network::quic::QuicConnectionManager;

/// Create a pair of connected QuicConnectionManagers on localhost.
async fn connected_pair() -> (QuicConnectionManager, QuicConnectionManager) {
    let a = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("alice".into()),
    )
    .await
    .unwrap();

    let b = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("bob".into()),
    )
    .await
    .unwrap();

    let b_addr = b.local_addr();
    let peer_id = a.connect_to(b_addr).await.unwrap();
    assert_eq!(peer_id, PeerId("bob".into()));

    // Allow time for accept loop to process the incoming connection
    tokio::time::sleep(Duration::from_millis(100)).await;

    (a, b)
}

#[tokio::test]
async fn two_peers_datagram_exchange() {
    let (a, b) = connected_pair().await;

    a.send_datagram(&PeerId("bob".into()), b"hello from alice")
        .await
        .unwrap();

    let (from, data) = b.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, b"hello from alice");

    b.send_datagram(&PeerId("alice".into()), b"hello from bob")
        .await
        .unwrap();

    let (from, data) = a.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("bob".into()));
    assert_eq!(data, b"hello from bob");
}

#[tokio::test]
async fn reliable_send_recv() {
    let (a, b) = connected_pair().await;

    a.send_reliable(&PeerId("bob".into()), b"reliable hello")
        .await
        .unwrap();

    let (from, data) = b.recv_reliable().await.unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, b"reliable hello");
}

#[tokio::test]
async fn multiple_concurrent_reliable() {
    let (a, b) = connected_pair().await;

    let bob = PeerId("bob".into());
    for i in 0..10u32 {
        let msg = format!("msg-{i}");
        a.send_reliable(&bob, msg.as_bytes()).await.unwrap();
    }

    let mut received = Vec::new();
    for _ in 0..10 {
        let (_from, data) = b.recv_reliable().await.unwrap();
        received.push(String::from_utf8(data).unwrap());
    }

    // All messages should arrive (order may vary due to concurrent stream processing)
    received.sort();
    let mut expected: Vec<String> = (0..10).map(|i| format!("msg-{i}")).collect();
    expected.sort();
    assert_eq!(received, expected);
}

#[tokio::test]
async fn connection_events_on_connect() {
    let a = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("alice".into()),
    )
    .await
    .unwrap();

    let b = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("bob".into()),
    )
    .await
    .unwrap();

    let mut events_a = a.subscribe();
    let mut events_b = b.subscribe();

    let b_addr = b.local_addr();
    a.connect_to(b_addr).await.unwrap();

    // Alice should see PeerConnected(bob)
    let event = events_a.recv().await.unwrap();
    assert!(matches!(event, ConnectionEvent::PeerConnected(id) if id == PeerId("bob".into())));

    // Bob should see PeerConnected(alice) from accept loop
    let event = tokio::time::timeout(Duration::from_secs(2), events_b.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, ConnectionEvent::PeerConnected(id) if id == PeerId("alice".into())));
}

#[tokio::test]
async fn connected_peers_list() {
    let a = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("alice".into()),
    )
    .await
    .unwrap();

    let b = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("bob".into()),
    )
    .await
    .unwrap();

    let c = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("carol".into()),
    )
    .await
    .unwrap();

    // Connect a→b and a→c
    a.connect_to(b.local_addr()).await.unwrap();
    a.connect_to(c.local_addr()).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut peers_a = a.connected_peers();
    peers_a.sort_by_key(|p| p.0.clone());
    assert_eq!(peers_a, vec![PeerId("bob".into()), PeerId("carol".into())]);

    // b should see alice (from accept)
    let peers_b = b.connected_peers();
    assert_eq!(peers_b, vec![PeerId("alice".into())]);
}

#[tokio::test]
async fn three_peer_full_mesh() {
    let a = QuicConnectionManager::new("127.0.0.1:0".parse().unwrap(), PeerId("a".into()))
        .await
        .unwrap();
    let b = QuicConnectionManager::new("127.0.0.1:0".parse().unwrap(), PeerId("b".into()))
        .await
        .unwrap();
    let c = QuicConnectionManager::new("127.0.0.1:0".parse().unwrap(), PeerId("c".into()))
        .await
        .unwrap();

    // Full mesh: a→b, a→c, b→c
    a.connect_to(b.local_addr()).await.unwrap();
    a.connect_to(c.local_addr()).await.unwrap();
    b.connect_to(c.local_addr()).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // a→b datagram
    a.send_datagram(&PeerId("b".into()), b"a-to-b").await.unwrap();
    let (from, data) = b.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("a".into()));
    assert_eq!(data, b"a-to-b");

    // b→c datagram
    b.send_datagram(&PeerId("c".into()), b"b-to-c").await.unwrap();
    let (from, data) = c.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("b".into()));
    assert_eq!(data, b"b-to-c");

    // c→a datagram
    c.send_datagram(&PeerId("a".into()), b"c-to-a").await.unwrap();
    let (from, data) = a.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("c".into()));
    assert_eq!(data, b"c-to-a");
}

#[tokio::test]
async fn large_reliable_message() {
    let (a, b) = connected_pair().await;

    // 1MB message
    let large_msg: Vec<u8> = (0..1_000_000).map(|i| (i % 256) as u8).collect();
    a.send_reliable(&PeerId("bob".into()), &large_msg)
        .await
        .unwrap();

    let (from, data) = b.recv_reliable().await.unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, large_msg);
}

#[tokio::test]
async fn send_to_disconnected_peer_fails() {
    let a = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("alice".into()),
    )
    .await
    .unwrap();

    let result = a
        .send_datagram(&PeerId("nobody".into()), b"hello")
        .await;
    assert!(matches!(result, Err(ConnectionError::PeerNotConnected(_))));

    let result = a
        .send_reliable(&PeerId("nobody".into()), b"hello")
        .await;
    assert!(matches!(result, Err(ConnectionError::PeerNotConnected(_))));
}

#[tokio::test]
async fn disconnect_event_on_close() {
    let a = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("alice".into()),
    )
    .await
    .unwrap();

    let b = QuicConnectionManager::new(
        "127.0.0.1:0".parse().unwrap(),
        PeerId("bob".into()),
    )
    .await
    .unwrap();

    let mut events_a = a.subscribe();
    a.connect_to(b.local_addr()).await.unwrap();

    // Wait for connect event
    let event = events_a.recv().await.unwrap();
    assert!(matches!(event, ConnectionEvent::PeerConnected(_)));

    // Close b's endpoint
    b.close();

    // Alice should eventually see PeerDisconnected
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_a.recv().await {
                Ok(ConnectionEvent::PeerDisconnected(id)) if id == PeerId("bob".into()) => {
                    return id;
                }
                _ => continue,
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(event, PeerId("bob".into()));
}
