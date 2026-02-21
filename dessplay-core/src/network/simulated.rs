//! Simulated network for deterministic testing.
//!
//! Provides a full peer mesh with configurable latency, packet loss, reordering,
//! and network partitions. Uses seeded RNG for reproducibility and tokio's paused
//! time for deterministic timing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;

use crate::protocol::{PeerControl, PeerDatagram};
use crate::types::PeerId;

use super::{MessageStream, Network, NetworkEvent};

/// Per-link configuration between two peers.
#[derive(Debug, Clone)]
pub struct LinkConfig {
    /// One-way latency.
    pub latency: Duration,
    /// Probability of dropping a datagram (0.0 = no loss, 1.0 = all lost).
    /// Only affects datagrams; control messages and streams are reliable.
    pub packet_loss: f64,
    /// Not yet implemented (reserved for Phase 9).
    pub reorder_window: usize,
    /// If true, all traffic between these peers is blocked.
    pub partitioned: bool,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            latency: Duration::from_millis(0),
            packet_loss: 0.0,
            reorder_window: 0,
            partitioned: false,
        }
    }
}

/// Shared state for the simulated network.
struct SimState {
    peers: HashMap<PeerId, PeerEntry>,
    links: HashMap<(PeerId, PeerId), LinkConfig>,
    default_config: LinkConfig,
    rng: StdRng,
    next_peer_id: u64,
}

struct PeerEntry {
    username: String,
    event_tx: mpsc::UnboundedSender<NetworkEvent>,
}

/// A simulated network mesh for testing.
///
/// All peers added via [`add_peer`] are automatically fully connected.
/// Network conditions can be configured per-link or globally.
pub struct SimulatedNetwork {
    state: Arc<Mutex<SimState>>,
}

impl SimulatedNetwork {
    /// Create a new simulated network with the given RNG seed.
    pub fn new(seed: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(SimState {
                peers: HashMap::new(),
                links: HashMap::new(),
                default_config: LinkConfig::default(),
                rng: StdRng::seed_from_u64(seed),
                next_peer_id: 1,
            })),
        }
    }

    /// Add a peer to the network. Returns a handle implementing [`Network`].
    ///
    /// The new peer is automatically connected to all existing peers (both
    /// directions get `PeerConnected` events).
    pub fn add_peer(&self, username: &str) -> SimPeerHandle {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        let peer_id = PeerId(state.next_peer_id);
        state.next_peer_id += 1;

        // Notify existing peers about the new peer
        let existing_ids: Vec<(PeerId, String)> = state
            .peers
            .iter()
            .map(|(id, entry)| (*id, entry.username.clone()))
            .collect();
        for (existing_id, existing_username) in &existing_ids {
            // Tell existing peer about new peer
            if let Some(existing) = state.peers.get(existing_id) {
                let _ = existing.event_tx.send(NetworkEvent::PeerConnected {
                    peer_id,
                    username: username.to_string(),
                });
            }
            // Tell new peer about existing peer
            let _ = event_tx.send(NetworkEvent::PeerConnected {
                peer_id: *existing_id,
                username: existing_username.clone(),
            });
        }

        state.peers.insert(
            peer_id,
            PeerEntry {
                username: username.to_string(),
                event_tx,
            },
        );

        SimPeerHandle {
            peer_id,
            state: Arc::clone(&self.state),
            event_rx: tokio::sync::Mutex::new(event_rx),
        }
    }

    /// Partition the link between two peers (both directions).
    pub fn partition(&self, a: PeerId, b: PeerId) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let default = state.default_config.clone();
        let link_ab = state.links.entry((a, b)).or_insert_with(|| default.clone());
        link_ab.partitioned = true;
        let link_ba = state.links.entry((b, a)).or_insert_with(|| default);
        link_ba.partitioned = true;

        // Send disconnect events
        if let Some(peer_a) = state.peers.get(&a) {
            let _ = peer_a
                .event_tx
                .send(NetworkEvent::PeerDisconnected { peer_id: b });
        }
        if let Some(peer_b) = state.peers.get(&b) {
            let _ = peer_b
                .event_tx
                .send(NetworkEvent::PeerDisconnected { peer_id: a });
        }
    }

    /// Heal the link between two peers (both directions).
    pub fn heal(&self, a: PeerId, b: PeerId) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        let was_partitioned_ab = state
            .links
            .get(&(a, b))
            .is_some_and(|l| l.partitioned);

        if let Some(link) = state.links.get_mut(&(a, b)) {
            link.partitioned = false;
        }
        if let Some(link) = state.links.get_mut(&(b, a)) {
            link.partitioned = false;
        }

        // Send reconnect events if they were partitioned
        if was_partitioned_ab {
            let username_a = state
                .peers
                .get(&a)
                .map(|p| p.username.clone())
                .unwrap_or_default();
            let username_b = state
                .peers
                .get(&b)
                .map(|p| p.username.clone())
                .unwrap_or_default();

            if let Some(peer_a) = state.peers.get(&a) {
                let _ = peer_a.event_tx.send(NetworkEvent::PeerConnected {
                    peer_id: b,
                    username: username_b,
                });
            }
            if let Some(peer_b) = state.peers.get(&b) {
                let _ = peer_b.event_tx.send(NetworkEvent::PeerConnected {
                    peer_id: a,
                    username: username_a,
                });
            }
        }
    }

    /// Set the link configuration for a specific direction (a → b).
    pub fn set_link(&self, a: PeerId, b: PeerId, config: LinkConfig) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.links.insert((a, b), config);
    }

    /// Set the default packet loss for all links (both existing and new).
    pub fn set_default_loss(&self, loss: f64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.default_config.packet_loss = loss;
    }
}

/// Per-peer handle that implements the [`Network`] trait.
pub struct SimPeerHandle {
    peer_id: PeerId,
    state: Arc<Mutex<SimState>>,
    event_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<NetworkEvent>>,
}

impl SimPeerHandle {
    /// This peer's ID.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    fn get_link_config(&self, to: PeerId) -> LinkConfig {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .links
            .get(&(self.peer_id, to))
            .cloned()
            .unwrap_or_else(|| state.default_config.clone())
    }

    fn should_drop_datagram(&self, to: PeerId) -> bool {
        let config = self.get_link_config(to);
        if config.partitioned {
            return true;
        }
        if config.packet_loss <= 0.0 {
            return false;
        }
        if config.packet_loss >= 1.0 {
            return true;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.rng.random_bool(config.packet_loss)
    }

    fn deliver_event(&self, to: PeerId, event: NetworkEvent) -> anyhow::Result<()> {
        let config = self.get_link_config(to);
        if config.partitioned {
            return Err(anyhow::anyhow!("link partitioned"));
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let peer = state
            .peers
            .get(&to)
            .ok_or_else(|| anyhow::anyhow!("peer {to:?} not found"))?;
        let tx = peer.event_tx.clone();
        let latency = config.latency;
        drop(state);

        if latency.is_zero() {
            tx.send(event)
                .map_err(|_| anyhow::anyhow!("peer {to:?} channel closed"))?;
        } else {
            tokio::spawn(async move {
                tokio::time::sleep(latency).await;
                let _ = tx.send(event);
            });
        }
        Ok(())
    }
}

impl Network for SimPeerHandle {
    async fn send_control(&self, peer: PeerId, msg: &PeerControl) -> anyhow::Result<()> {
        self.deliver_event(
            peer,
            NetworkEvent::PeerControl {
                from: self.peer_id,
                message: msg.clone(),
            },
        )
    }

    async fn send_datagram(&self, peer: PeerId, msg: &PeerDatagram) -> anyhow::Result<()> {
        if self.should_drop_datagram(peer) {
            return Ok(()); // Silently dropped, like real UDP
        }
        self.deliver_event(
            peer,
            NetworkEvent::PeerDatagram {
                from: self.peer_id,
                message: msg.clone(),
            },
        )
    }

    async fn open_stream(&self, peer: PeerId) -> anyhow::Result<MessageStream> {
        let config = self.get_link_config(peer);
        if config.partitioned {
            return Err(anyhow::anyhow!("link partitioned"));
        }

        // Create a pair of duplex streams
        let (a_send, b_recv) = tokio::io::duplex(64 * 1024);
        let (b_send, a_recv) = tokio::io::duplex(64 * 1024);

        let our_stream = MessageStream {
            send: Box::new(a_send),
            recv: Box::new(a_recv),
        };
        let their_stream = MessageStream {
            send: Box::new(b_send),
            recv: Box::new(b_recv),
        };

        self.deliver_event(
            peer,
            NetworkEvent::IncomingStream {
                from: self.peer_id,
                stream: their_stream,
            },
        )?;

        Ok(our_stream)
    }

    async fn recv(&self) -> anyhow::Result<NetworkEvent> {
        let mut rx = self.event_rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("network shut down"))
    }

    fn connected_peers(&self) -> Vec<PeerId> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .peers
            .keys()
            .filter(|id| {
                **id != self.peer_id
                    && !state
                        .links
                        .get(&(self.peer_id, **id))
                        .is_some_and(|l| l.partitioned)
            })
            .copied()
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::protocol::{CrdtOp, LwwValue};
    use crate::types::{UserState, UserId};

    #[tokio::test]
    async fn peer_connect_events() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Alice should get a PeerConnected for Bob
        let event = alice.recv().await?;
        match event {
            NetworkEvent::PeerConnected { peer_id, username } => {
                assert_eq!(peer_id, bob.peer_id());
                assert_eq!(username, "bob");
            }
            other => panic!("expected PeerConnected, got {other:?}"),
        }

        // Bob should get a PeerConnected for Alice
        let event = bob.recv().await?;
        match event {
            NetworkEvent::PeerConnected { peer_id, username } => {
                assert_eq!(peer_id, alice.peer_id());
                assert_eq!(username, "alice");
            }
            other => panic!("expected PeerConnected, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn control_delivery() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        let msg = PeerControl::Hello {
            peer_id: crate::types::PeerId(0),
            username: "alice".into(),
        };
        alice.send_control(bob.peer_id(), &msg).await?;

        let event = bob.recv().await?;
        match event {
            NetworkEvent::PeerControl { from, message } => {
                assert_eq!(from, alice.peer_id());
                assert_eq!(message, msg);
            }
            other => panic!("expected PeerControl, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn datagram_delivery() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        let msg = PeerDatagram::Position {
            timestamp: 1000,
            position_secs: 42.5,
        };
        alice.send_datagram(bob.peer_id(), &msg).await?;

        let event = bob.recv().await?;
        match event {
            NetworkEvent::PeerDatagram { from, message } => {
                assert_eq!(from, alice.peer_id());
                assert_eq!(message, msg);
            }
            other => panic!("expected PeerDatagram, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn packet_loss_statistics() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        // Set 50% packet loss
        net.set_link(
            alice.peer_id(),
            bob.peer_id(),
            LinkConfig {
                packet_loss: 0.5,
                ..Default::default()
            },
        );

        let total = 1000;
        for i in 0..total {
            let msg = PeerDatagram::Position {
                timestamp: i,
                position_secs: i as f64,
            };
            alice.send_datagram(bob.peer_id(), &msg).await?;
        }

        // Count how many arrived (with a timeout for each)
        let mut received = 0u32;
        while let Ok(Ok(NetworkEvent::PeerDatagram { .. })) =
            tokio::time::timeout(Duration::from_millis(10), bob.recv()).await
        {
            received += 1;
        }

        // With 50% loss over 1000 packets, expect ~400-600 delivered
        assert!(
            (350..=650).contains(&received),
            "expected ~500 delivered, got {received}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn partition_blocks_traffic() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        net.partition(alice.peer_id(), bob.peer_id());

        // Drain disconnect events
        let event = alice.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerDisconnected { .. }));
        let event = bob.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerDisconnected { .. }));

        // Control messages should fail
        let msg = PeerControl::Hello {
            peer_id: crate::types::PeerId(0),
            username: "alice".into(),
        };
        let result = alice.send_control(bob.peer_id(), &msg).await;
        assert!(result.is_err());

        // Datagrams should be silently dropped (not error)
        let msg = PeerDatagram::Position {
            timestamp: 1,
            position_secs: 1.0,
        };
        alice.send_datagram(bob.peer_id(), &msg).await?;

        // Verify nothing arrives at bob
        let result = tokio::time::timeout(Duration::from_millis(50), bob.recv()).await;
        assert!(result.is_err(), "should timeout — nothing should arrive");

        Ok(())
    }

    #[tokio::test]
    async fn heal_restores_traffic() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain initial connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        // Partition
        net.partition(alice.peer_id(), bob.peer_id());
        let _ = alice.recv().await?; // disconnect
        let _ = bob.recv().await?; // disconnect

        // Heal
        net.heal(alice.peer_id(), bob.peer_id());

        // Should get reconnect events
        let event = alice.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerConnected { .. }));
        let event = bob.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerConnected { .. }));

        // Traffic should work again
        let msg = PeerControl::Hello {
            peer_id: crate::types::PeerId(0),
            username: "test".into(),
        };
        alice.send_control(bob.peer_id(), &msg).await?;
        let event = bob.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerControl { .. }));

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn latency_with_paused_time() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        // Set 100ms latency
        net.set_link(
            alice.peer_id(),
            bob.peer_id(),
            LinkConfig {
                latency: Duration::from_millis(100),
                ..Default::default()
            },
        );

        let msg = PeerControl::Hello {
            peer_id: crate::types::PeerId(0),
            username: "delayed".into(),
        };
        alice.send_control(bob.peer_id(), &msg).await?;

        // Should not arrive immediately
        let result = tokio::time::timeout(Duration::from_millis(50), bob.recv()).await;
        assert!(result.is_err(), "should not arrive before latency expires");

        // Advance time past latency
        tokio::time::sleep(Duration::from_millis(60)).await;
        let event = bob.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerControl { .. }));

        Ok(())
    }

    #[tokio::test]
    async fn stream_open_and_use() -> anyhow::Result<()> {
        use crate::framing::{read_framed, write_framed, TAG_PEER_CONTROL};

        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        // Alice opens stream to Bob
        let mut alice_stream = alice.open_stream(bob.peer_id()).await?;

        // Bob receives the incoming stream
        let event = bob.recv().await?;
        let mut bob_stream = match event {
            NetworkEvent::IncomingStream { from, stream } => {
                assert_eq!(from, alice.peer_id());
                stream
            }
            other => panic!("expected IncomingStream, got {other:?}"),
        };

        // Alice writes, Bob reads
        let msg = PeerControl::Hello {
            peer_id: crate::types::PeerId(0),
            username: "via stream".into(),
        };
        write_framed(&mut alice_stream.send, TAG_PEER_CONTROL, &msg).await?;

        let decoded: Option<PeerControl> =
            read_framed(&mut bob_stream.recv, TAG_PEER_CONTROL).await?;
        assert_eq!(decoded, Some(msg));

        // Bob writes back, Alice reads
        let reply = PeerControl::Hello {
            peer_id: crate::types::PeerId(0),
            username: "reply".into(),
        };
        write_framed(&mut bob_stream.send, TAG_PEER_CONTROL, &reply).await?;

        let decoded: Option<PeerControl> =
            read_framed(&mut alice_stream.recv, TAG_PEER_CONTROL).await?;
        assert_eq!(decoded, Some(reply));

        Ok(())
    }

    #[tokio::test]
    async fn connected_peers_list() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");
        let charlie = net.add_peer("charlie");

        // Drain events
        while let Ok(Ok(_)) =
            tokio::time::timeout(Duration::from_millis(10), alice.recv()).await
        {
        }

        let mut peers = alice.connected_peers();
        peers.sort();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&bob.peer_id()));
        assert!(peers.contains(&charlie.peer_id()));

        // Partition Alice-Bob
        net.partition(alice.peer_id(), bob.peer_id());
        let peers = alice.connected_peers();
        assert_eq!(peers.len(), 1);
        assert!(peers.contains(&charlie.peer_id()));

        Ok(())
    }

    #[tokio::test]
    async fn three_peer_mesh() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");
        let charlie = net.add_peer("charlie");

        // Drain connect events
        while let Ok(Ok(_)) =
            tokio::time::timeout(Duration::from_millis(10), alice.recv()).await
        {
        }
        while let Ok(Ok(_)) =
            tokio::time::timeout(Duration::from_millis(10), bob.recv()).await
        {
        }
        while let Ok(Ok(_)) =
            tokio::time::timeout(Duration::from_millis(10), charlie.recv()).await
        {
        }

        // Alice sends to Bob
        let msg = PeerControl::StateOp {
            op: CrdtOp::LwwWrite {
                timestamp: 1,
                value: LwwValue::UserState(UserId("alice".into()), UserState::Ready),
            },
        };
        alice.send_control(bob.peer_id(), &msg).await?;
        let event = bob.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerControl { .. }));

        // Bob forwards to Charlie
        bob.send_control(charlie.peer_id(), &msg).await?;
        let event = charlie.recv().await?;
        assert!(matches!(event, NetworkEvent::PeerControl { .. }));

        Ok(())
    }

    #[tokio::test]
    async fn datagram_with_crdt_op() -> anyhow::Result<()> {
        let net = SimulatedNetwork::new(42);
        let alice = net.add_peer("alice");
        let bob = net.add_peer("bob");

        // Drain connect events
        let _ = alice.recv().await?;
        let _ = bob.recv().await?;

        let msg = PeerDatagram::StateOp {
            op: CrdtOp::ChatAppend {
                user_id: UserId("alice".into()),
                seq: 1,
                timestamp: 1000,
                text: "hello!".into(),
            },
        };
        alice.send_datagram(bob.peer_id(), &msg).await?;

        let event = bob.recv().await?;
        match event {
            NetworkEvent::PeerDatagram { from, message } => {
                assert_eq!(from, alice.peer_id());
                assert_eq!(message, msg);
            }
            other => panic!("expected PeerDatagram, got {other:?}"),
        }

        Ok(())
    }
}
