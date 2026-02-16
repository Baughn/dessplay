use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Duration;

use super::wire::{self, ClockSyncMessage, WireMessage};
use super::{ConnectionError, ConnectionEvent, ConnectionManager, PeerId};

/// A microsecond-resolution timestamp on the shared clock.
///
/// All peers converge to the same shared clock via the NTP-like sync protocol.
/// Timestamps are comparable across peers (within sync tolerance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SharedTimestamp(pub i64);

impl SharedTimestamp {
    pub fn as_micros(self) -> i64 {
        self.0
    }
}

impl std::ops::Sub for SharedTimestamp {
    type Output = i64;
    fn sub(self, rhs: Self) -> i64 {
        self.0 - rhs.0
    }
}

impl std::fmt::Display for SharedTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secs = self.0 / 1_000_000;
        let us = (self.0 % 1_000_000).abs();
        write!(f, "{secs}.{us:06}")
    }
}

/// Reference epoch for converting between tokio::time::Instant and microseconds.
/// Using tokio's Instant so it works with `tokio::time::pause()` in tests.
static EPOCH: std::sync::OnceLock<tokio::time::Instant> = std::sync::OnceLock::new();

fn local_monotonic_us() -> i64 {
    let epoch = EPOCH.get_or_init(tokio::time::Instant::now);
    tokio::time::Instant::now().duration_since(*epoch).as_micros() as i64
}

/// A clock synchronized across peers.
///
/// Maintains an offset from the local monotonic clock. `now()` is lock-free
/// (uses `AtomicI64`) and safe to call from any context.
#[derive(Clone)]
pub struct SharedClock {
    offset: Arc<AtomicI64>,
}

impl SharedClock {
    fn new() -> Self {
        Self {
            offset: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Current shared time.
    pub fn now(&self) -> SharedTimestamp {
        let local = local_monotonic_us();
        let offset = self.offset.load(Ordering::Relaxed);
        SharedTimestamp(local + offset)
    }

    /// Current offset in microseconds (for debugging/testing).
    pub fn offset_us(&self) -> i64 {
        self.offset.load(Ordering::Relaxed)
    }
}

/// Rolling buffer of NTP-like clock offset samples for one peer.
struct SampleBuffer {
    offsets: VecDeque<i64>,
    rtts: VecDeque<i64>,
    capacity: usize,
}

impl SampleBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            offsets: VecDeque::with_capacity(capacity),
            rtts: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn add_sample(&mut self, offset_us: i64, rtt_us: i64) {
        if self.offsets.len() >= self.capacity {
            self.offsets.pop_front();
            self.rtts.pop_front();
        }
        self.offsets.push_back(offset_us);
        self.rtts.push_back(rtt_us);
    }

    /// Median of stored offsets. Median filters outliers from jitter.
    fn median_offset(&self) -> Option<i64> {
        if self.offsets.is_empty() {
            return None;
        }
        let mut sorted: Vec<i64> = self.offsets.iter().copied().collect();
        sorted.sort();
        Some(sorted[sorted.len() / 2])
    }
}

/// Clock synchronization service.
///
/// Takes ownership of the datagram receive path on the `ConnectionManager`.
/// Upper layers call `recv_app_datagram()` instead of `conn.recv_datagram()`.
///
/// Runs NTP-like ping/pong exchanges with all connected peers and maintains
/// a `SharedClock` with the computed aggregate offset.
pub struct ClockSyncService {
    clock: SharedClock,
    conn: Arc<dyn ConnectionManager>,
    samples: Arc<Mutex<HashMap<PeerId, SampleBuffer>>>,
    app_datagram_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<(PeerId, Vec<u8>)>>,
    app_datagram_tx: mpsc::UnboundedSender<(PeerId, Vec<u8>)>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    ping_interval: Duration,
    sample_capacity: usize,
}

impl ClockSyncService {
    /// Create a new clock sync service wrapping the given connection manager.
    ///
    /// `ping_interval` controls how often pings are sent to each peer.
    /// Call `start()` to begin the background tasks.
    pub fn new(conn: Arc<dyn ConnectionManager>, ping_interval: Duration) -> Arc<Self> {
        let (app_datagram_tx, app_datagram_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            clock: SharedClock::new(),
            conn,
            samples: Arc::new(Mutex::new(HashMap::new())),
            app_datagram_rx: tokio::sync::Mutex::new(app_datagram_rx),
            app_datagram_tx,
            tasks: Mutex::new(Vec::new()),
            ping_interval,
            sample_capacity: 11,
        })
    }

    /// Get a cloneable handle to the shared clock.
    pub fn clock(&self) -> SharedClock {
        self.clock.clone()
    }

    /// Start the background ping sender and datagram dispatcher tasks.
    pub fn start(self: &Arc<Self>) {
        let mut tasks = self.tasks.lock().unwrap();

        // Ping sender task
        tasks.push(tokio::spawn({
            let this = Arc::clone(self);
            async move {
                this.ping_loop().await;
            }
        }));

        // Datagram dispatcher task
        tasks.push(tokio::spawn({
            let this = Arc::clone(self);
            async move {
                this.dispatch_loop().await;
            }
        }));
    }

    /// Receive the next application-level datagram (clock sync messages filtered out).
    pub async fn recv_app_datagram(&self) -> Result<(PeerId, Vec<u8>), ConnectionError> {
        let mut rx = self.app_datagram_rx.lock().await;
        rx.recv().await.ok_or(ConnectionError::Closed)
    }

    /// Send an application datagram (wraps in `WireMessage::Application`).
    pub async fn send_app_datagram(
        &self,
        peer: &PeerId,
        data: &[u8],
    ) -> Result<(), ConnectionError> {
        let msg = wire::encode(&WireMessage::Application(data.to_vec()));
        self.conn.send_datagram(peer, &msg).await
    }

    /// Send data reliably (passthrough to underlying ConnectionManager).
    pub async fn send_reliable(
        &self,
        peer: &PeerId,
        data: &[u8],
    ) -> Result<(), ConnectionError> {
        self.conn.send_reliable(peer, data).await
    }

    /// Receive the next reliable message (passthrough).
    pub async fn recv_reliable(&self) -> Result<(PeerId, Vec<u8>), ConnectionError> {
        self.conn.recv_reliable().await
    }

    /// Get the underlying connection manager's connected peers.
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.conn.connected_peers()
    }

    /// Subscribe to connection events (passthrough to ConnectionManager).
    pub fn subscribe(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.conn.subscribe()
    }

    // --- Internal ---

    async fn ping_loop(&self) {
        loop {
            let peers = self.conn.connected_peers();
            for peer in peers {
                let t1 = local_monotonic_us();
                let msg = wire::encode(&WireMessage::ClockSync(ClockSyncMessage::Ping {
                    t1_us: t1,
                }));
                let _ = self.conn.send_datagram(&peer, &msg).await;
            }
            tokio::time::sleep(self.ping_interval).await;
        }
    }

    async fn dispatch_loop(&self) {
        loop {
            match self.conn.recv_datagram().await {
                Ok((peer, data)) => match wire::decode(&data) {
                    Ok(WireMessage::ClockSync(msg)) => {
                        self.handle_clock_message(&peer, msg).await;
                    }
                    Ok(WireMessage::Application(payload)) => {
                        let _ = self.app_datagram_tx.send((peer, payload));
                    }
                    Err(e) => {
                        tracing::warn!("failed to decode wire message from {peer}: {e}");
                    }
                },
                Err(ConnectionError::Closed) => break,
                Err(e) => {
                    tracing::warn!("datagram recv error: {e}");
                }
            }
        }
    }

    async fn handle_clock_message(&self, from: &PeerId, msg: ClockSyncMessage) {
        match msg {
            ClockSyncMessage::Ping { t1_us } => {
                let t2 = local_monotonic_us();
                let t3 = local_monotonic_us();
                let pong = wire::encode(&WireMessage::ClockSync(ClockSyncMessage::Pong {
                    t1_us,
                    t2_us: t2,
                    t3_us: t3,
                }));
                let _ = self.conn.send_datagram(from, &pong).await;
            }
            ClockSyncMessage::Pong {
                t1_us,
                t2_us,
                t3_us,
            } => {
                let t4 = local_monotonic_us();
                let offset = ((t2_us - t1_us) + (t3_us - t4)) / 2;
                let rtt = (t4 - t1_us) - (t3_us - t2_us);

                // Discard unreasonable samples
                if !(0..=5_000_000).contains(&rtt) {
                    tracing::debug!(
                        peer = %from, rtt_us = rtt,
                        "discarding clock sample with unreasonable RTT"
                    );
                    return;
                }

                tracing::trace!(
                    peer = %from, offset_us = offset, rtt_us = rtt,
                    "clock sync sample"
                );

                let mut samples = self.samples.lock().unwrap();
                let buffer = samples
                    .entry(from.clone())
                    .or_insert_with(|| SampleBuffer::new(self.sample_capacity));
                buffer.add_sample(offset, rtt);

                self.recompute_offset(&samples);
            }
        }
    }

    fn recompute_offset(&self, samples: &HashMap<PeerId, SampleBuffer>) {
        let offsets: Vec<i64> = samples
            .values()
            .filter_map(|buf| buf.median_offset())
            .collect();

        if offsets.is_empty() {
            return;
        }

        let avg = offsets.iter().sum::<i64>() / offsets.len() as i64;
        self.clock.offset.store(avg, Ordering::Relaxed);
    }
}

impl Drop for ClockSyncService {
    fn drop(&mut self) {
        let tasks = self.tasks.lock().unwrap();
        for task in tasks.iter() {
            task.abort();
        }
    }
}
