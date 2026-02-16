mod common;

use std::sync::Arc;

use dessplay::network::clock::ClockSyncService;
use dessplay::network::{ConnectionManager, PeerId};
use tokio::time::Duration;

use common::simulated_network::SimulatedNetwork;

/// Wait for clock sync to stabilize by advancing simulated time.
async fn advance_sync_rounds(rounds: u32, interval: Duration) {
    for _ in 0..rounds {
        tokio::time::sleep(interval + Duration::from_millis(1)).await;
        // Extra sleep to allow pong responses to be delivered and processed
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(start_paused = true)]
async fn converge_zero_latency() {
    let net = SimulatedNetwork::new(42);
    let a = Arc::new(net.add_peer(PeerId("a".into())));
    let b = Arc::new(net.add_peer(PeerId("b".into())));

    let interval = Duration::from_millis(100);
    let svc_a = ClockSyncService::new(a as Arc<dyn ConnectionManager>, interval);
    let svc_b = ClockSyncService::new(b as Arc<dyn ConnectionManager>, interval);
    svc_a.start();
    svc_b.start();

    // Let several sync rounds complete
    advance_sync_rounds(15, interval).await;

    let clock_a = svc_a.clock();
    let clock_b = svc_b.clock();

    let diff = (clock_a.now() - clock_b.now()).abs();
    assert!(
        diff < 1000, // within 1ms
        "clocks should converge with zero latency, diff={diff}us"
    );
}

#[tokio::test(start_paused = true)]
async fn converge_symmetric_latency() {
    let net = SimulatedNetwork::new(42);
    let alice = PeerId("alice".into());
    let bob = PeerId("bob".into());
    let a = Arc::new(net.add_peer(alice.clone()));
    let b = Arc::new(net.add_peer(bob.clone()));

    // 50ms symmetric latency
    net.set_link_symmetric(
        &alice,
        &bob,
        common::simulated_network::LinkConfig {
            latency_ms: 50,
            jitter_ms: 0,
            loss_rate: 0.0,
            reorder_rate: 0.0,
        },
    );

    let interval = Duration::from_millis(200);
    let svc_a = ClockSyncService::new(a as Arc<dyn ConnectionManager>, interval);
    let svc_b = ClockSyncService::new(b as Arc<dyn ConnectionManager>, interval);
    svc_a.start();
    svc_b.start();

    // With symmetric latency, the offset should converge well
    advance_sync_rounds(15, interval).await;

    let clock_a = svc_a.clock();
    let clock_b = svc_b.clock();

    let diff = (clock_a.now() - clock_b.now()).abs();
    assert!(
        diff < 5000, // within 5ms
        "clocks should converge with symmetric latency, diff={diff}us"
    );
}

#[tokio::test(start_paused = true)]
async fn jitter_filtering() {
    let net = SimulatedNetwork::new(42);
    let alice = PeerId("alice".into());
    let bob = PeerId("bob".into());
    let a = Arc::new(net.add_peer(alice.clone()));
    let b = Arc::new(net.add_peer(bob.clone()));

    // High jitter: 50ms ± 40ms
    net.set_link_symmetric(
        &alice,
        &bob,
        common::simulated_network::LinkConfig {
            latency_ms: 50,
            jitter_ms: 40,
            loss_rate: 0.0,
            reorder_rate: 0.0,
        },
    );

    let interval = Duration::from_millis(100);
    let svc_a = ClockSyncService::new(a as Arc<dyn ConnectionManager>, interval);
    let svc_b = ClockSyncService::new(b as Arc<dyn ConnectionManager>, interval);
    svc_a.start();
    svc_b.start();

    // Need more rounds for median filter to stabilize with jitter
    advance_sync_rounds(20, interval).await;

    let clock_a = svc_a.clock();
    let clock_b = svc_b.clock();

    let diff = (clock_a.now() - clock_b.now()).abs();
    // With jitter, we expect somewhat larger error but still reasonable
    assert!(
        diff < 100_000, // within 100ms
        "clocks should converge despite jitter, diff={diff}us"
    );
}

#[tokio::test(start_paused = true)]
async fn three_peers_agree() {
    let net = SimulatedNetwork::new(42);
    let a_id = PeerId("a".into());
    let b_id = PeerId("b".into());
    let c_id = PeerId("c".into());
    let a = Arc::new(net.add_peer(a_id.clone()));
    let b = Arc::new(net.add_peer(b_id.clone()));
    let c = Arc::new(net.add_peer(c_id.clone()));

    // Different latencies for each link
    let config_10 = common::simulated_network::LinkConfig {
        latency_ms: 10,
        ..Default::default()
    };
    let config_20 = common::simulated_network::LinkConfig {
        latency_ms: 20,
        ..Default::default()
    };
    let config_15 = common::simulated_network::LinkConfig {
        latency_ms: 15,
        ..Default::default()
    };
    net.set_link_symmetric(&a_id, &b_id, config_10);
    net.set_link_symmetric(&b_id, &c_id, config_20);
    net.set_link_symmetric(&a_id, &c_id, config_15);

    let interval = Duration::from_millis(100);
    let svc_a = ClockSyncService::new(a as Arc<dyn ConnectionManager>, interval);
    let svc_b = ClockSyncService::new(b as Arc<dyn ConnectionManager>, interval);
    let svc_c = ClockSyncService::new(c as Arc<dyn ConnectionManager>, interval);
    svc_a.start();
    svc_b.start();
    svc_c.start();

    advance_sync_rounds(15, interval).await;

    let clock_a = svc_a.clock();
    let clock_b = svc_b.clock();
    let clock_c = svc_c.clock();

    let ab_diff = (clock_a.now() - clock_b.now()).abs();
    let bc_diff = (clock_b.now() - clock_c.now()).abs();
    let ac_diff = (clock_a.now() - clock_c.now()).abs();

    assert!(
        ab_diff < 5000,
        "a-b clocks should agree, diff={ab_diff}us"
    );
    assert!(
        bc_diff < 5000,
        "b-c clocks should agree, diff={bc_diff}us"
    );
    assert!(
        ac_diff < 5000,
        "a-c clocks should agree, diff={ac_diff}us"
    );
}

#[tokio::test(start_paused = true)]
async fn offset_stabilizes() {
    let net = SimulatedNetwork::new(42);
    let alice = PeerId("alice".into());
    let bob = PeerId("bob".into());
    let a = Arc::new(net.add_peer(alice.clone()));
    let b = Arc::new(net.add_peer(bob.clone()));

    net.set_link_symmetric(
        &alice,
        &bob,
        common::simulated_network::LinkConfig {
            latency_ms: 30,
            ..Default::default()
        },
    );

    let interval = Duration::from_millis(100);
    let svc_a = ClockSyncService::new(a as Arc<dyn ConnectionManager>, interval);
    svc_a.start();

    let svc_b = ClockSyncService::new(b as Arc<dyn ConnectionManager>, interval);
    svc_b.start();

    // Collect offsets over time, verify they stabilize
    let mut offsets = Vec::new();
    for _ in 0..20 {
        tokio::time::sleep(interval + Duration::from_millis(50)).await;
        offsets.push(svc_a.clock().offset_us());
    }

    // Last 5 offsets should be close together (stabilized)
    let last_5 = &offsets[offsets.len() - 5..];
    let min = *last_5.iter().min().unwrap();
    let max = *last_5.iter().max().unwrap();
    let spread = max - min;
    assert!(
        spread < 5000, // within 5ms spread
        "offset should stabilize, last_5={last_5:?}, spread={spread}us"
    );
}

#[tokio::test(start_paused = true)]
async fn peer_disconnect_continues_sync() {
    let net = SimulatedNetwork::new(42);
    let a_id = PeerId("a".into());
    let b_id = PeerId("b".into());
    let c_id = PeerId("c".into());
    let a = Arc::new(net.add_peer(a_id.clone()));
    let b = Arc::new(net.add_peer(b_id.clone()));
    let c = Arc::new(net.add_peer(c_id.clone()));

    let interval = Duration::from_millis(100);
    let svc_a = ClockSyncService::new(a as Arc<dyn ConnectionManager>, interval);
    let svc_b = ClockSyncService::new(b as Arc<dyn ConnectionManager>, interval);
    let svc_c = ClockSyncService::new(c as Arc<dyn ConnectionManager>, interval);
    svc_a.start();
    svc_b.start();
    svc_c.start();

    advance_sync_rounds(10, interval).await;

    // Partition c from everyone
    net.partition(&c_id, &a_id);
    net.partition(&c_id, &b_id);
    net.partition(&a_id, &c_id);
    net.partition(&b_id, &c_id);

    // a and b should continue syncing fine
    advance_sync_rounds(10, interval).await;

    let diff = (svc_a.clock().now() - svc_b.clock().now()).abs();
    assert!(
        diff < 2000,
        "a-b should still sync after c disconnects, diff={diff}us"
    );
}

#[tokio::test(start_paused = true)]
async fn app_datagrams_pass_through() {
    let net = SimulatedNetwork::new(42);
    let alice = PeerId("alice".into());
    let bob = PeerId("bob".into());
    let a = Arc::new(net.add_peer(alice.clone()));
    let b = Arc::new(net.add_peer(bob.clone()));

    let interval = Duration::from_secs(10); // slow pings so they don't interfere
    let svc_a = ClockSyncService::new(a as Arc<dyn ConnectionManager>, interval);
    let svc_b = ClockSyncService::new(b as Arc<dyn ConnectionManager>, interval);
    svc_a.start();
    svc_b.start();

    // Send an application datagram through the service
    svc_a
        .send_app_datagram(&bob, b"hello app")
        .await
        .unwrap();

    let (from, data) = svc_b.recv_app_datagram().await.unwrap();
    assert_eq!(from, alice);
    assert_eq!(data, b"hello app");

    // Send in reverse
    svc_b
        .send_app_datagram(&alice, b"response")
        .await
        .unwrap();

    let (from, data) = svc_a.recv_app_datagram().await.unwrap();
    assert_eq!(from, bob);
    assert_eq!(data, b"response");
}
