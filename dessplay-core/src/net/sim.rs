//! In-process simulated transport (feature `test-support`).
//!
//! Implements the [`Transport`]/[`Connector`]/[`Listener`] traits over
//! tokio channels with configurable per-link conditions: latency,
//! datagram loss, datagram jitter (which produces reordering), bandwidth,
//! and partitions. Designed for `tokio::time::pause()` — all delays are
//! tokio timers, so simulated seconds cost microseconds and tests are
//! reproducible from the RNG seed.
//!
//! Semantics:
//! - **Control frames** are reliable and ordered (a partition delays
//!   them; nothing drops them), like a QUIC stream.
//! - **Datagrams** suffer loss and jitter, and a partition drops them.
//! - **Bandwidth** is modeled as serialization delay shared per link
//!   direction (control and datagrams share the budget).
//! - `close()` delivers a `Closed` event to both ends (the peer's after
//!   one latency); **dropping** a transport without closing is silent
//!   death — the peer sees nothing, which is exactly what presence
//!   timeouts exist to catch.
//!
//! Known limitation: bytes inside an open [`BiStream`] bypass the link
//! simulation (they ride a `tokio::io::duplex`). Stream *establishment*
//! respects partitions and latency. Revisit when Phase 9 needs
//! bandwidth-accurate transfer tests.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::transport::{BiStream, Connector, Listener, Transport, TransportError, TransportEvent};

/// A named endpoint in the simulated network.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EndpointId(pub String);

impl EndpointId {
    /// Construct from anything string-like.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// Conditions on one (symmetric) link.
#[derive(Clone, Debug)]
pub struct LinkConfig {
    /// One-way propagation delay.
    pub latency: Duration,
    /// Probability a datagram is dropped, 0.0..=1.0.
    pub datagram_loss: f64,
    /// Random extra delay 0..jitter per datagram — produces reordering.
    pub datagram_jitter: Duration,
    /// Link throughput in bytes/sec; `None` = infinite.
    pub bandwidth: Option<u64>,
    /// While true: control frames are held, datagrams are dropped.
    pub partitioned: bool,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            latency: Duration::ZERO,
            datagram_loss: 0.0,
            datagram_jitter: Duration::ZERO,
            bandwidth: None,
            partitioned: false,
        }
    }
}

/// Default simulated datagram payload limit, mirroring typical QUIC.
pub const SIM_MAX_DATAGRAM: usize = 1200;

type ConnId = u64;
type Pending = Vec<(TransportEvent, usize)>;
/// One reliable event scheduled for ordered delivery: its due time, the
/// receiving inbox, and the event itself.
type ReliableItem = (
    Instant,
    mpsc::UnboundedSender<TransportEvent>,
    TransportEvent,
);

struct NetState {
    rng: StdRng,
    links: HashMap<(EndpointId, EndpointId), LinkConfig>,
    default_link: LinkConfig,
    listeners: HashMap<EndpointId, mpsc::UnboundedSender<(SimTransport, SocketAddr)>>,
    /// Inbound event sender per (connection, receiving endpoint).
    senders: HashMap<(ConnId, EndpointId), mpsc::UnboundedSender<TransportEvent>>,
    /// Bandwidth watermark per (connection, sending endpoint).
    clear_at: HashMap<(ConnId, EndpointId), Instant>,
    /// Control frames held by a partition, per (connection, sender).
    pending: HashMap<(ConnId, EndpointId), Pending>,
    /// Ordered delivery pump per (connection, sender): every reliable
    /// event of a direction flows through one task, so "control frames
    /// are reliable and ordered" holds by construction. A per-frame
    /// task race once broke it under a busy multi-thread scheduler
    /// (see `control_order_survives_scheduler_churn`).
    pumps: HashMap<(ConnId, EndpointId), mpsc::UnboundedSender<ReliableItem>>,
    addrs: HashMap<EndpointId, SocketAddr>,
    next_conn: ConnId,
    max_datagram: usize,
}

/// Handle to the simulated network. Cheap to clone.
#[derive(Clone)]
pub struct SimNetwork {
    state: Arc<Mutex<NetState>>,
}

fn pair_key(a: &EndpointId, b: &EndpointId) -> (EndpointId, EndpointId) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

impl SimNetwork {
    /// A fresh network. Every random decision derives from `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(NetState {
                rng: StdRng::seed_from_u64(seed),
                links: HashMap::new(),
                default_link: LinkConfig::default(),
                listeners: HashMap::new(),
                senders: HashMap::new(),
                clear_at: HashMap::new(),
                pending: HashMap::new(),
                pumps: HashMap::new(),
                addrs: HashMap::new(),
                next_conn: 0,
                max_datagram: SIM_MAX_DATAGRAM,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, NetState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Configure the link between two endpoints (symmetric).
    pub fn set_link(&self, a: &EndpointId, b: &EndpointId, config: LinkConfig) {
        let partitioned = config.partitioned;
        self.lock().links.insert(pair_key(a, b), config);
        if !partitioned {
            self.flush_pending(a, b);
        }
    }

    /// The default config for links never passed to [`set_link`].
    pub fn set_default_link(&self, config: LinkConfig) {
        self.lock().default_link = config;
    }

    /// Partition or heal a link, preserving its other settings.
    pub fn set_partitioned(&self, a: &EndpointId, b: &EndpointId, partitioned: bool) {
        {
            let mut state = self.lock();
            let key = pair_key(a, b);
            let mut config = state
                .links
                .get(&key)
                .cloned()
                .unwrap_or_else(|| state.default_link.clone());
            config.partitioned = partitioned;
            state.links.insert(key, config);
        }
        if !partitioned {
            self.flush_pending(a, b);
        }
    }

    /// Override the simulated datagram size limit.
    pub fn set_max_datagram(&self, max: usize) {
        self.lock().max_datagram = max;
    }

    /// Kill every connection between two endpoints: both ends receive a
    /// `Closed` event immediately (think middlebox reset, not silence —
    /// for silence, partition instead). Reconnecting afterward works.
    pub fn disconnect(&self, a: &EndpointId, b: &EndpointId) {
        let mut state = self.lock();
        // A connection involves both endpoints under the same ConnId:
        // find those, then kill both ends.
        let conns: Vec<ConnId> = state
            .senders
            .keys()
            .filter(|(conn, endpoint)| {
                endpoint == a && state.senders.contains_key(&(*conn, b.clone()))
            })
            .map(|(conn, _)| *conn)
            .collect();
        for conn in conns {
            for endpoint in [a, b] {
                if let Some(tx) = state.senders.remove(&(conn, endpoint.clone())) {
                    let _ = tx.send(TransportEvent::Closed {
                        reason: "connection killed".into(),
                    });
                }
            }
        }
    }

    /// Create the listener for an endpoint. One per endpoint.
    pub fn listener(&self, endpoint: &EndpointId) -> SimListener {
        let (tx, rx) = mpsc::unbounded_channel();
        self.lock().listeners.insert(endpoint.clone(), tx);
        SimListener {
            incoming: tokio::sync::Mutex::new(rx),
        }
    }

    /// Create a connector dialing `to` as `from`.
    pub fn connector(&self, from: &EndpointId, to: &EndpointId) -> SimConnector {
        SimConnector {
            net: self.clone(),
            from: from.clone(),
            to: to.clone(),
        }
    }

    fn addr_of(state: &mut NetState, endpoint: &EndpointId) -> SocketAddr {
        let next_index = state.addrs.len() as u16;
        *state.addrs.entry(endpoint.clone()).or_insert_with(|| {
            SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                40_000_u16.saturating_add(next_index),
            )
        })
    }

    fn link_config(state: &NetState, a: &EndpointId, b: &EndpointId) -> LinkConfig {
        state
            .links
            .get(&pair_key(a, b))
            .cloned()
            .unwrap_or_else(|| state.default_link.clone())
    }

    /// Schedule delivery of an event from `from` toward its peer on
    /// `conn`. Returns false if the event was dropped (datagram loss or
    /// partition).
    fn deliver(
        &self,
        conn: ConnId,
        from: &EndpointId,
        to: &EndpointId,
        event: TransportEvent,
        len: usize,
        is_datagram: bool,
    ) -> bool {
        let mut state = self.lock();
        let link = Self::link_config(&state, from, to);

        if link.partitioned {
            if is_datagram {
                return false;
            }
            state
                .pending
                .entry((conn, from.clone()))
                .or_default()
                .push((event, len));
            return true;
        }

        if is_datagram && state.rng.random_bool(link.datagram_loss.clamp(0.0, 1.0)) {
            return false;
        }
        let jitter = if is_datagram && !link.datagram_jitter.is_zero() {
            link.datagram_jitter.mul_f64(state.rng.random::<f64>())
        } else {
            Duration::ZERO
        };

        let Some(target) = state.senders.get(&(conn, to.clone())).cloned() else {
            return false;
        };

        let now = Instant::now();
        let key = (conn, from.clone());
        let clear = state.clear_at.get(&key).copied().unwrap_or(now).max(now);
        let transmit = match link.bandwidth {
            Some(bw) if bw > 0 => Duration::from_secs_f64(len as f64 / bw as f64),
            _ => Duration::ZERO,
        };
        let ready = clear + transmit;
        state.clear_at.insert(key.clone(), ready);
        let deliver_at = ready + link.latency + jitter;

        if is_datagram {
            // Datagrams may race each other (loss and jitter already
            // reorder them); a task per frame is fine.
            drop(state);
            tokio::spawn(async move {
                tokio::time::sleep_until(deliver_at).await;
                let _ = target.send(event);
            });
        } else {
            // Reliable events are serialized through the direction's
            // pump — enqueued under the state lock, so delivery order
            // matches scheduling order no matter how the runtime
            // interleaves tasks. A later frame with a shorter latency
            // still waits its turn: head-of-line blocking is exactly
            // an ordered stream's semantics.
            let pump = state.pumps.entry(key).or_insert_with(|| {
                let (tx, mut rx) = mpsc::unbounded_channel::<ReliableItem>();
                tokio::spawn(async move {
                    while let Some((at, target, event)) = rx.recv().await {
                        tokio::time::sleep_until(at).await;
                        let _ = target.send(event);
                    }
                });
                tx
            });
            let _ = pump.send((deliver_at, target, event));
        }
        true
    }

    /// Re-schedule everything a partition was holding on both directions
    /// of a link.
    fn flush_pending(&self, a: &EndpointId, b: &EndpointId) {
        let held: Vec<(ConnId, EndpointId, EndpointId, Pending)> = {
            let mut state = self.lock();
            let keys: Vec<(ConnId, EndpointId)> = state
                .pending
                .keys()
                .filter(|(_, from)| from == a || from == b)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| {
                    let queue = state.pending.remove(&key)?;
                    let (conn, from) = key;
                    let to = if from == *a { b.clone() } else { a.clone() };
                    Some((conn, from, to, queue))
                })
                .collect()
        };
        for (conn, from, to, queue) in held {
            for (event, len) in queue {
                self.deliver(conn, &from, &to, event, len, false);
            }
        }
    }
}

/// One end of a simulated connection.
pub struct SimTransport {
    net: SimNetwork,
    conn: ConnId,
    local: EndpointId,
    peer: EndpointId,
    inbound: tokio::sync::Mutex<mpsc::UnboundedReceiver<TransportEvent>>,
    closed: Arc<AtomicBool>,
}

impl SimTransport {
    fn check_open(&self) -> Result<(), TransportError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError::ConnectionLost("closed".into()));
        }
        Ok(())
    }
}

impl Transport for SimTransport {
    async fn send_control(&self, frame: &[u8]) -> Result<(), TransportError> {
        self.check_open()?;
        self.net.deliver(
            self.conn,
            &self.local,
            &self.peer,
            TransportEvent::Control(frame.to_vec()),
            frame.len(),
            false,
        );
        Ok(())
    }

    async fn send_datagram(&self, frame: &[u8]) -> Result<(), TransportError> {
        self.check_open()?;
        let max = self.net.lock().max_datagram;
        if frame.len() > max {
            return Err(TransportError::DatagramTooLarge {
                len: frame.len(),
                max,
            });
        }
        self.net.deliver(
            self.conn,
            &self.local,
            &self.peer,
            TransportEvent::Datagram(frame.to_vec()),
            frame.len(),
            true,
        );
        Ok(())
    }

    fn max_datagram_size(&self) -> Option<usize> {
        Some(self.net.lock().max_datagram)
    }

    async fn open_stream(&self) -> Result<BiStream, TransportError> {
        self.check_open()?;
        let (local_side, remote_side) = duplex_bistreams();
        self.net.deliver(
            self.conn,
            &self.local,
            &self.peer,
            TransportEvent::IncomingStream(remote_side),
            64,
            false,
        );
        Ok(local_side)
    }

    async fn recv(&self) -> Result<TransportEvent, TransportError> {
        let mut inbound = self.inbound.lock().await;
        match inbound.recv().await {
            Some(event) => {
                if matches!(event, TransportEvent::Closed { .. }) {
                    self.closed.store(true, Ordering::SeqCst);
                }
                Ok(event)
            }
            None => Err(TransportError::ConnectionLost("simulated peer gone".into())),
        }
    }

    async fn close(&self, reason: &str) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // Tell the peer (subject to latency/partition), then tear down
        // our own inbox so a local recv() unblocks.
        self.net.deliver(
            self.conn,
            &self.local,
            &self.peer,
            TransportEvent::Closed {
                reason: reason.to_string(),
            },
            16,
            false,
        );
        let mut state = self.net.lock();
        if let Some(tx) = state.senders.remove(&(self.conn, self.local.clone())) {
            let _ = tx.send(TransportEvent::Closed {
                reason: format!("locally closed: {reason}"),
            });
        }
    }
}

fn duplex_bistreams() -> (BiStream, BiStream) {
    let (a, b) = tokio::io::duplex(256 * 1024);
    let (a_read, a_write) = tokio::io::split(a);
    let (b_read, b_write) = tokio::io::split(b);
    (
        BiStream {
            send: Box::new(a_write),
            recv: Box::new(a_read),
        },
        BiStream {
            send: Box::new(b_write),
            recv: Box::new(b_read),
        },
    )
}

/// Dials a simulated endpoint.
pub struct SimConnector {
    net: SimNetwork,
    from: EndpointId,
    to: EndpointId,
}

impl Connector for SimConnector {
    type Conn = SimTransport;

    async fn connect(&self) -> Result<SimTransport, TransportError> {
        let (client, server, client_addr, listener) = {
            let mut state = self.net.lock();
            let listener =
                state.listeners.get(&self.to).cloned().ok_or_else(|| {
                    TransportError::Setup(format!("no listener at {:?}", self.to.0))
                })?;
            let conn = state.next_conn;
            state.next_conn += 1;

            let (client_tx, client_rx) = mpsc::unbounded_channel();
            let (server_tx, server_rx) = mpsc::unbounded_channel();
            state.senders.insert((conn, self.from.clone()), client_tx);
            state.senders.insert((conn, self.to.clone()), server_tx);
            let client_addr = SimNetwork::addr_of(&mut state, &self.from);

            let client = SimTransport {
                net: self.net.clone(),
                conn,
                local: self.from.clone(),
                peer: self.to.clone(),
                inbound: tokio::sync::Mutex::new(client_rx),
                closed: Arc::new(AtomicBool::new(false)),
            };
            let server = SimTransport {
                net: self.net.clone(),
                conn,
                local: self.to.clone(),
                peer: self.from.clone(),
                inbound: tokio::sync::Mutex::new(server_rx),
                closed: Arc::new(AtomicBool::new(false)),
            };
            (client, server, client_addr, listener)
        };

        listener
            .send((server, client_addr))
            .map_err(|_| TransportError::Setup("listener dropped".into()))?;
        Ok(client)
    }
}

/// Accepts simulated connections for one endpoint.
pub struct SimListener {
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<(SimTransport, SocketAddr)>>,
}

impl Listener for SimListener {
    type Conn = SimTransport;

    async fn accept(&self) -> Result<(SimTransport, SocketAddr), TransportError> {
        self.incoming
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| TransportError::Setup("network gone".into()))
    }
}
