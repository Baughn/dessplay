use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Duration, sleep};

use dessplay::network::{ConnectionError, ConnectionEvent, ConnectionManager, PeerId};

/// Per-link configuration controlling simulated network behavior.
#[derive(Debug, Clone)]
pub struct LinkConfig {
    /// Base one-way latency in milliseconds.
    pub latency_ms: u64,
    /// Uniform jitter around base latency in milliseconds.
    pub jitter_ms: u64,
    /// Datagram drop probability [0.0, 1.0]. Only applies to datagrams.
    pub loss_rate: f64,
    /// Probability of out-of-order delivery [0.0, 1.0]. Only applies to datagrams.
    pub reorder_rate: f64,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            latency_ms: 0,
            jitter_ms: 0,
            loss_rate: 0.0,
            reorder_rate: 0.0,
        }
    }
}

/// Reason a message was dropped in the simulation.
#[derive(Debug, Clone, Copy)]
pub enum DropReason {
    Partitioned,
    RandomLoss,
}

/// Trace event for debugging network test failures.
#[derive(Debug, Clone)]
pub enum NetworkTraceEvent {
    MessageSent {
        from: PeerId,
        to: PeerId,
        payload_type: &'static str,
    },
    MessageDelivered {
        from: PeerId,
        to: PeerId,
        delay_ms: u64,
    },
    MessageDropped {
        from: PeerId,
        to: PeerId,
        reason: DropReason,
    },
}

/// Shared state inside the SimulatedNetwork.
struct NetworkInner {
    rng: StdRng,
    /// Per-link config: (from, to) → LinkConfig. Missing = default (perfect).
    link_configs: HashMap<(PeerId, PeerId), LinkConfig>,
    /// Blocked links (partitions).
    partitions: std::collections::HashSet<(PeerId, PeerId)>,
    /// Datagram delivery channels per peer.
    datagram_txs: HashMap<PeerId, mpsc::UnboundedSender<(PeerId, Vec<u8>)>>,
    /// Reliable delivery channels per peer.
    reliable_txs: HashMap<PeerId, mpsc::UnboundedSender<(PeerId, Vec<u8>)>>,
    /// Connection event broadcaster.
    event_tx: broadcast::Sender<ConnectionEvent>,
    /// All registered peers.
    peers: Vec<PeerId>,
}

/// A simulated network for testing. Creates peers and controls topology.
///
/// Compatible with `tokio::time::pause()` for fast deterministic tests.
pub struct SimulatedNetwork {
    inner: Arc<Mutex<NetworkInner>>,
}

impl SimulatedNetwork {
    /// Create a new simulated network with a deterministic seed.
    pub fn new(seed: u64) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Mutex::new(NetworkInner {
                rng: StdRng::seed_from_u64(seed),
                link_configs: HashMap::new(),
                partitions: std::collections::HashSet::new(),
                datagram_txs: HashMap::new(),
                reliable_txs: HashMap::new(),
                event_tx,
                peers: Vec::new(),
            })),
        }
    }

    /// Add a peer and return its ConnectionManager handle.
    pub fn add_peer(&self, id: PeerId) -> SimulatedConnectionManager {
        let (datagram_tx, datagram_rx) = mpsc::unbounded_channel();
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();

        let mut inner = self.inner.lock().unwrap();

        // Notify existing peers about new connection
        for existing in &inner.peers {
            let _ = inner.event_tx.send(ConnectionEvent::PeerConnected(id.clone()));
            let _ = inner
                .event_tx
                .send(ConnectionEvent::PeerConnected(existing.clone()));
        }

        inner.datagram_txs.insert(id.clone(), datagram_tx);
        inner.reliable_txs.insert(id.clone(), reliable_tx);
        inner.peers.push(id.clone());

        let event_rx = inner.event_tx.subscribe();

        SimulatedConnectionManager {
            id,
            network: Arc::clone(&self.inner),
            datagram_rx: tokio::sync::Mutex::new(datagram_rx),
            reliable_rx: tokio::sync::Mutex::new(reliable_rx),
            event_tx: inner.event_tx.clone(),
            event_rx: tokio::sync::Mutex::new(event_rx),
        }
    }

    /// Configure the link from `from` to `to` (one direction only).
    pub fn set_link(&self, from: &PeerId, to: &PeerId, config: LinkConfig) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .link_configs
            .insert((from.clone(), to.clone()), config);
    }

    /// Configure the link in both directions.
    pub fn set_link_symmetric(&self, a: &PeerId, b: &PeerId, config: LinkConfig) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .link_configs
            .insert((a.clone(), b.clone()), config.clone());
        inner.link_configs.insert((b.clone(), a.clone()), config);
    }

    /// Block all traffic from `from` to `to`.
    pub fn partition(&self, from: &PeerId, to: &PeerId) {
        let mut inner = self.inner.lock().unwrap();
        inner.partitions.insert((from.clone(), to.clone()));
        let _ = inner.event_tx.send(ConnectionEvent::PeerDisconnected(to.clone()));
    }

    /// Restore traffic from `from` to `to`.
    pub fn heal(&self, from: &PeerId, to: &PeerId) {
        let mut inner = self.inner.lock().unwrap();
        inner.partitions.remove(&(from.clone(), to.clone()));
        let _ = inner.event_tx.send(ConnectionEvent::PeerConnected(to.clone()));
    }
}

/// A peer's handle into the SimulatedNetwork. Implements `ConnectionManager`.
pub struct SimulatedConnectionManager {
    id: PeerId,
    network: Arc<Mutex<NetworkInner>>,
    datagram_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<(PeerId, Vec<u8>)>>,
    reliable_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<(PeerId, Vec<u8>)>>,
    event_tx: broadcast::Sender<ConnectionEvent>,
    event_rx: tokio::sync::Mutex<broadcast::Receiver<ConnectionEvent>>,
}

impl SimulatedConnectionManager {
    /// Compute delay for a message on a given link, consuming RNG state.
    fn compute_delay_and_should_drop(
        inner: &mut NetworkInner,
        from: &PeerId,
        to: &PeerId,
        is_datagram: bool,
    ) -> Result<Duration, DropReason> {
        let config = inner
            .link_configs
            .get(&(from.clone(), to.clone()))
            .cloned()
            .unwrap_or_default();

        // Check partition
        if inner.partitions.contains(&(from.clone(), to.clone())) {
            return Err(DropReason::Partitioned);
        }

        // For datagrams, check random loss
        if is_datagram && config.loss_rate > 0.0 {
            let roll: f64 = inner.rng.random();
            if roll < config.loss_rate {
                return Err(DropReason::RandomLoss);
            }
        }

        // Compute delay
        let base = config.latency_ms as i64;
        let jitter = if config.jitter_ms > 0 {
            inner
                .rng
                .random_range(-(config.jitter_ms as i64)..=(config.jitter_ms as i64))
        } else {
            0
        };
        let mut delay_ms = (base + jitter).max(0) as u64;

        // For datagrams, check reorder (adds extra delay)
        if is_datagram && config.reorder_rate > 0.0 {
            let roll: f64 = inner.rng.random();
            if roll < config.reorder_rate {
                let extra = inner.rng.random_range(0..=(2 * config.latency_ms).max(10));
                delay_ms += extra;
            }
        }

        Ok(Duration::from_millis(delay_ms))
    }
}

#[async_trait]
impl ConnectionManager for SimulatedConnectionManager {
    async fn send_datagram(&self, peer: &PeerId, data: &[u8]) -> Result<(), ConnectionError> {
        let (delay, tx) = {
            let mut inner = self.network.lock().unwrap();

            let delay =
                Self::compute_delay_and_should_drop(&mut inner, &self.id, peer, true)
                    .map_err(|reason| match reason {
                        DropReason::Partitioned => {
                            tracing::debug!(
                                from = %self.id, to = %peer,
                                "datagram dropped: partitioned"
                            );
                            // Datagrams silently fail on partition (unreliable)
                            return ConnectionError::Partitioned(peer.clone());
                        }
                        DropReason::RandomLoss => {
                            tracing::debug!(
                                from = %self.id, to = %peer,
                                "datagram dropped: random loss"
                            );
                            return ConnectionError::Partitioned(peer.clone());
                        }
                    });

            // For datagrams, silently succeed on drop (unreliable semantics)
            let delay = match delay {
                Ok(d) => d,
                Err(_) => return Ok(()), // silently dropped
            };

            let tx = inner
                .datagram_txs
                .get(peer)
                .cloned()
                .ok_or_else(|| ConnectionError::PeerNotConnected(peer.clone()))?;

            (delay, tx)
        };

        // Schedule delayed delivery
        let from = self.id.clone();
        let to = peer.clone();
        let data = data.to_vec();
        tokio::spawn(async move {
            if delay > Duration::ZERO {
                sleep(delay).await;
            }
            let _ = tx.send((from.clone(), data));
            tracing::debug!(
                from = %from, to = %to, delay_ms = delay.as_millis() as u64,
                "datagram delivered"
            );
        });

        Ok(())
    }

    async fn recv_datagram(&self) -> Result<(PeerId, Vec<u8>), ConnectionError> {
        let mut rx = self.datagram_rx.lock().await;
        rx.recv().await.ok_or(ConnectionError::Closed)
    }

    async fn send_reliable(&self, peer: &PeerId, data: &[u8]) -> Result<(), ConnectionError> {
        let (delay, tx) = {
            let mut inner = self.network.lock().unwrap();

            // Check partition — reliable sends fail explicitly
            if inner.partitions.contains(&(self.id.clone(), peer.clone())) {
                return Err(ConnectionError::Partitioned(peer.clone()));
            }

            let delay =
                Self::compute_delay_and_should_drop(&mut inner, &self.id, peer, false)
                    .map_err(|_| ConnectionError::Partitioned(peer.clone()))?;

            let tx = inner
                .reliable_txs
                .get(peer)
                .cloned()
                .ok_or_else(|| ConnectionError::PeerNotConnected(peer.clone()))?;

            (delay, tx)
        };

        let from = self.id.clone();
        let to = peer.clone();
        let data = data.to_vec();
        tokio::spawn(async move {
            if delay > Duration::ZERO {
                sleep(delay).await;
            }
            let _ = tx.send((from.clone(), data));
            tracing::debug!(
                from = %from, to = %to, delay_ms = delay.as_millis() as u64,
                "reliable message delivered"
            );
        });

        Ok(())
    }

    async fn recv_reliable(&self) -> Result<(PeerId, Vec<u8>), ConnectionError> {
        let mut rx = self.reliable_rx.lock().await;
        rx.recv().await.ok_or(ConnectionError::Closed)
    }

    fn subscribe(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.event_tx.subscribe()
    }

    fn connected_peers(&self) -> Vec<PeerId> {
        let inner = self.network.lock().unwrap();
        inner
            .peers
            .iter()
            .filter(|p| {
                **p != self.id && !inner.partitions.contains(&(self.id.clone(), (*p).clone()))
            })
            .cloned()
            .collect()
    }
}
