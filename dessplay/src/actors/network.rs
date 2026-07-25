//! The network actor: owns the connection to the rendezvous server.
//!
//! Phase 3 scope: connect, authenticate, keep the clock synced, surface
//! peer-list updates, reconnect with a fixed backoff. State sync
//! traffic (ops, snapshots, merges) is surfaced as events from Phase 4.
//!
//! The actor is generic over [`Connector`], so the simulation harness
//! runs it over `SimConnector` and production over `QuicConnector`. The
//! local clock is injected for the same reason.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dessplay_core::net::framing::{read_frame, write_frame};
use dessplay_core::net::timesync::TimeSync;
use dessplay_core::net::{
    Connector, LinkStats, PROTOCOL_VERSION, PeerId, PeerMessage, RelayEnvelope, Role,
    ServerControl, Transport, TransportError, TransportEvent, WireMessage,
};
use dessplay_core::types::{Epoch, UserId};
use dessplay_core::wire;
use tokio::sync::mpsc;

/// How the actor reads the local clock (unix millis). Injected so tests
/// can drive it from paused tokio time.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Commands from the main loop.
#[derive(Debug)]
pub enum NetworkCommand {
    /// Send on the control stream only (reliable, ordered).
    SendReliable(Box<ServerControl>),
    /// Send on the control stream *and* eagerly as a datagram when it
    /// fits (the latency optimization for state ops).
    SendEager(Box<ServerControl>),
    /// Send as a datagram only; silently dropped if datagrams are
    /// unavailable (playback positions — the 1s reliable tick covers
    /// the gap).
    SendDatagramOnly(Box<ServerControl>),
    /// Send a file-transfer message to a peer, relayed through the
    /// server on the dedicated relay stream. Dropped if the relay
    /// stream isn't up yet (pre-auth) — transfer logic retries.
    SendPeer {
        /// The destination peer.
        to: PeerId,
        /// The message.
        message: Box<PeerMessage>,
    },
    /// Close the connection and exit the actor.
    Shutdown,
}

impl NetworkCommand {
    /// Terse description for logs (Debug-formatting a command can dump
    /// an entire state merge).
    fn name(&self) -> &'static str {
        match self {
            NetworkCommand::SendReliable(msg) => msg.variant_name(),
            NetworkCommand::SendEager(msg) => msg.variant_name(),
            NetworkCommand::SendDatagramOnly(msg) => msg.variant_name(),
            NetworkCommand::SendPeer { .. } => "SendPeer",
            NetworkCommand::Shutdown => "Shutdown",
        }
    }
}

/// Events to the main loop.
#[derive(Debug)]
pub enum NetworkEvent {
    /// A connection attempt is starting (initial or reconnect). Feeds
    /// the status bar's link indicator — a dead handshake can take the
    /// full per-address timeout ladder, and silence there reads as a
    /// hang (design.md UI principles: no silent long-running work).
    Connecting {
        /// 1-based attempt counter, reset only by process restart.
        attempt: u64,
    },
    /// Authenticated; the server saw us at this address.
    Connected {
        /// Our address as observed by the server.
        observed_addr: SocketAddr,
    },
    /// The server refused us admission (bad password, or a protocol
    /// version mismatch). Terminal — the actor exits without retrying.
    Rejected {
        /// Human-readable refusal, shown to the user verbatim.
        message: String,
    },
    /// A fresh peer list.
    PeerList {
        /// Present/Lost/Departed peers this session knows about.
        peers: Vec<dessplay_core::net::PeerInfo>,
        /// Known usernames not currently in `peers` (design.md #15).
        known_offline: Vec<dessplay_core::net::KnownUser>,
    },
    /// A state-sync message from the server (op, merge, snapshot,
    /// hash). Routed to the sync actor; `via_datagram` selects the
    /// FIFO-guarded apply path.
    Server {
        /// The message.
        msg: Box<ServerControl>,
        /// True if it arrived as a datagram (unordered path).
        via_datagram: bool,
    },
    /// The clock offset estimate changed.
    ClockSync {
        /// Server-minus-local offset, milliseconds.
        offset_millis: i64,
    },
    /// Results for an AniDB name search we sent.
    SearchResults {
        /// The query these results answer (stale replies are dropped
        /// by the UI, which knows the current query).
        query: String,
        /// Best matches.
        results: Vec<dessplay_core::net::AniDbSearchHit>,
    },
    /// A file-transfer message relayed from a peer.
    Peer {
        /// The originating peer.
        from: PeerId,
        /// The message.
        message: Box<PeerMessage>,
    },
    /// Connection lost; the actor will retry.
    Disconnected {
        /// Human-readable cause.
        reason: String,
    },
    /// A once-a-second connection-health sample, emitted while
    /// authenticated. Feeds the status field's health display
    /// (design.md, Connection Health Line).
    LinkHealth(LinkHealthReport),
}

/// One second's worth of connection health, measured entirely inside
/// the actor so it is transport-agnostic (deterministic under the sim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkHealthReport {
    /// Median time-sync probe round-trip, once any probe was answered.
    pub rtt_millis: Option<u64>,
    /// Consecutive steady-state probes (30s apart) sent without any
    /// answer arriving in between. The initial 200ms seeding burst is
    /// excluded — its probes are legitimately answered late.
    pub unanswered_probes: u32,
    /// Milliseconds since the last frame of any kind arrived from the
    /// server. The server broadcasts a `StateHash` at least every 30s,
    /// so a healthy link keeps this well under ~35s; a large value on a
    /// live connection means sync is dead even though QUIC is not.
    pub server_silence_millis: u64,
    /// Bytes/sec sent over roughly the last second (control stream,
    /// datagrams, and relayed transfer traffic).
    pub tx_bps: u64,
    /// Bytes/sec received over roughly the last second.
    pub rx_bps: u64,
    /// Transport-level statistics when available (QUIC only; `None` on
    /// the sim). Display supplement — never an input to gating or
    /// health classification.
    pub quic: Option<LinkStats>,
}

/// Cumulative per-connection byte counters, covering the control
/// stream, datagrams, and the relay stream in both directions. Shared
/// with the spawned relay reader task, hence atomics; created fresh per
/// connection so rate deltas reset naturally on reconnect.
#[derive(Debug, Default)]
struct NetCounters {
    tx_bytes: AtomicU64,
    rx_bytes: AtomicU64,
}

impl NetCounters {
    fn add_tx(&self, bytes: usize) {
        self.tx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn add_rx(&self, bytes: usize) {
        self.rx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

/// Static actor configuration.
pub struct NetworkConfig {
    /// Our username.
    pub username: UserId,
    /// The shared room password.
    pub password: String,
    /// Interactive or seeder.
    pub role: Role,
    /// Current epoch, shared with the sync actor (it advances on
    /// snapshot adoption; reconnect auths use the live value).
    pub epoch: Arc<AtomicU64>,
    /// Local clock, unix millis.
    pub clock: Clock,
    /// Steady-state probe interval (30s in production).
    pub time_sync_interval: Duration,
    /// Delay between reconnection attempts.
    pub reconnect_backoff: Duration,
    /// The version declared in `Auth`. Always [`PROTOCOL_VERSION`] in
    /// production; overridable so tests can drive the real actor into
    /// the server's mismatch refusal.
    pub protocol_version: u32,
}

impl NetworkConfig {
    /// Production timing defaults.
    pub fn new(
        username: UserId,
        password: String,
        role: Role,
        epoch: Arc<AtomicU64>,
        clock: Clock,
    ) -> Self {
        Self {
            username,
            password,
            role,
            epoch,
            clock,
            time_sync_interval: Duration::from_secs(30),
            reconnect_backoff: Duration::from_secs(2),
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

/// Probes sent back-to-back right after connecting, to seed the offset
/// window before the steady-state cadence takes over.
const INITIAL_PROBE_BURST: u32 = 5;
/// Spacing of the initial burst.
const BURST_INTERVAL: Duration = Duration::from_millis(200);
/// Cadence of [`NetworkEvent::LinkHealth`] samples while authenticated.
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);

/// Run the network actor until shutdown or auth failure.
pub async fn run<C: Connector>(
    connector: Arc<C>,
    config: NetworkConfig,
    mut commands: mpsc::Receiver<NetworkCommand>,
    events: mpsc::Sender<NetworkEvent>,
) {
    let mut attempt_number: u64 = 0;
    loop {
        attempt_number += 1;
        let attempt_started = tokio::time::Instant::now();
        tracing::info!(attempt = attempt_number, "connecting to server");
        let _ = events
            .send(NetworkEvent::Connecting {
                attempt: attempt_number,
            })
            .await;
        // A connection attempt to an unreachable host can take the full
        // handshake timeout (30s of QUIC retries) — Shutdown must
        // interrupt it, or quitting hangs on this actor. Other commands
        // arriving mid-attempt are discarded, like sends during the
        // reconnect backoff (the upward merge heals the gap).
        let attempt = tokio::select! {
            attempt = connector.connect() => attempt,
            _ = async {
                loop {
                    match commands.recv().await {
                        Some(NetworkCommand::Shutdown) | None => return,
                        Some(discarded) => {
                            tracing::debug!(
                                cmd = discarded.name(),
                                "discarding command while connecting"
                            );
                        }
                    }
                }
            } => return,
        };
        match attempt {
            Ok(conn) => {
                tracing::info!(
                    attempt = attempt_number,
                    elapsed_ms = attempt_started.elapsed().as_millis() as u64,
                    "transport connected"
                );
                match run_connection(&conn, &config, &mut commands, &events).await {
                    ConnectionEnd::Shutdown => {
                        conn.close("shutting down").await;
                        return;
                    }
                    ConnectionEnd::Rejected(message) => {
                        tracing::warn!("server refused us: {message}");
                        let _ = events.send(NetworkEvent::Rejected { message }).await;
                        return;
                    }
                    ConnectionEnd::Lost(reason) => {
                        let _ = events.send(NetworkEvent::Disconnected { reason }).await;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    attempt = attempt_number,
                    elapsed_ms = attempt_started.elapsed().as_millis() as u64,
                    error = %e,
                    "connection attempt failed"
                );
                let _ = events
                    .send(NetworkEvent::Disconnected {
                        reason: e.to_string(),
                    })
                    .await;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(config.reconnect_backoff) => {}
            cmd = commands.recv() => {
                if matches!(cmd, Some(NetworkCommand::Shutdown) | None) {
                    return;
                }
            }
        }
    }
}

enum ConnectionEnd {
    Shutdown,
    /// Terminal refusal (bad password, protocol mismatch): no retry.
    Rejected(String),
    Lost(String),
}

/// Send a control message, encoding it first.
async fn send_control<T: Transport>(
    conn: &T,
    counters: &NetCounters,
    msg: &ServerControl,
) -> Result<(), TransportError> {
    let frame = wire::encode(&WireMessage::Control(msg.clone()))
        .map_err(|e| TransportError::Setup(format!("encode: {e}")))?;
    // No size for Auth: its frame length leaks the password length.
    if matches!(msg, ServerControl::Auth { .. }) {
        tracing::trace!(msg = "Auth", "send control");
    } else {
        tracing::trace!(
            msg = msg.variant_name(),
            bytes = frame.len(),
            "send control"
        );
    }
    counters.add_tx(frame.len());
    conn.send_control(&frame).await
}

/// Send a small message as a datagram, falling back to the control
/// stream when datagrams are unavailable.
async fn send_datagram_or_control<T: Transport>(
    conn: &T,
    counters: &NetCounters,
    msg: &ServerControl,
) -> Result<(), TransportError> {
    let frame = wire::encode(&WireMessage::Control(msg.clone()))
        .map_err(|e| TransportError::Setup(format!("encode: {e}")))?;
    tracing::trace!(
        msg = msg.variant_name(),
        bytes = frame.len(),
        "send datagram"
    );
    counters.add_tx(frame.len());
    match conn.send_datagram(&frame).await {
        Err(TransportError::DatagramUnsupported | TransportError::DatagramTooLarge { .. }) => {
            conn.send_control(&frame).await
        }
        other => other,
    }
}

/// Send reliably, plus eagerly as a datagram when it fits (the size
/// rule): receivers dedup, so the datagram is pure latency win.
async fn send_eager<T: Transport>(
    conn: &T,
    counters: &NetCounters,
    msg: &ServerControl,
) -> Result<(), TransportError> {
    let frame = wire::encode(&WireMessage::Control(msg.clone()))
        .map_err(|e| TransportError::Setup(format!("encode: {e}")))?;
    tracing::trace!(msg = msg.variant_name(), bytes = frame.len(), "send eager");
    counters.add_tx(frame.len());
    conn.send_control(&frame).await?;
    if conn
        .max_datagram_size()
        .is_some_and(|max| frame.len() <= max)
    {
        counters.add_tx(frame.len());
        let _ = conn.send_datagram(&frame).await;
    }
    Ok(())
}

/// Read the relay stream, surfacing each `Forwarded` peer message as a
/// [`NetworkEvent::Peer`]. Exits when the stream closes (connection
/// gone); a fresh connection opens a new relay stream and reader.
async fn run_relay_reader(
    mut recv: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    events: mpsc::Sender<NetworkEvent>,
    counters: Arc<NetCounters>,
) {
    loop {
        let frame = match read_frame(&mut recv).await {
            Ok(frame) => frame,
            Err(_) => break,
        };
        counters.add_rx(frame.len());
        match wire::decode::<RelayEnvelope>(&frame) {
            Ok(RelayEnvelope::Forwarded { from, message }) => match wire::decode(&message) {
                Ok(message) => {
                    let _ = events
                        .send(NetworkEvent::Peer {
                            from,
                            message: Box::new(message),
                        })
                        .await;
                }
                Err(e) => tracing::warn!("undecodable peer message: {e}"),
            },
            // The server only ever sends Forwarded; ignore the rest.
            Ok(RelayEnvelope::Forward { .. } | RelayEnvelope::Hello) => {}
            Err(e) => tracing::warn!("undecodable relay envelope: {e}"),
        }
    }
    tracing::trace!("relay reader exiting");
}

async fn run_connection<T: Transport>(
    conn: &T,
    config: &NetworkConfig,
    commands: &mut mpsc::Receiver<NetworkCommand>,
    events: &mpsc::Sender<NetworkEvent>,
) -> ConnectionEnd {
    let counters = Arc::new(NetCounters::default());

    // ---- Authenticate.
    let auth = ServerControl::Auth {
        username: config.username.clone(),
        password: config.password.clone(),
        role: config.role,
        epoch: Epoch(config.epoch.load(Ordering::SeqCst)),
        protocol_version: config.protocol_version,
    };
    if let Err(e) = send_control(conn, &counters, &auth).await {
        return ConnectionEnd::Lost(e.to_string());
    }
    tracing::debug!(user = %config.username.0, role = ?config.role, "auth sent");

    let mut timesync = TimeSync::new();
    let mut probes_sent: u32 = 0;
    let mut last_offset: Option<i64> = None;
    let mut authenticated = false;
    let mut next_probe = tokio::time::Instant::now(); // first probe right after AuthOk
    // The relay stream's write half; opened after AuthOk (the server
    // drops pre-auth streams). `None` until then / after a relay error.
    let mut relay_send: Option<Box<dyn tokio::io::AsyncWrite + Send + Unpin>> = None;

    // ---- Health bookkeeping (LinkHealth samples). All measured with
    // local monotonic Instants — never shared-clock timestamps, whose
    // cross-machine comparison is unsound (types.rs, SharedTimestamp).
    let mut probe_outstanding = false;
    let mut probes_since_answer: u32 = 0;
    let mut last_server_frame = tokio::time::Instant::now();
    let mut health_tick = tokio::time::interval(HEALTH_INTERVAL);
    health_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_health = tokio::time::Instant::now();
    let mut last_tx: u64 = 0;
    let mut last_rx: u64 = 0;

    loop {
        tokio::select! {
            event = conn.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(e) => return ConnectionEnd::Lost(e.to_string()),
                };
                let (payload, via_datagram) = match event {
                    TransportEvent::Control(bytes) => (bytes, false),
                    TransportEvent::Datagram(bytes) => (bytes, true),
                    TransportEvent::IncomingStream(_) => continue, // Phase 9
                    TransportEvent::Closed { reason } => return ConnectionEnd::Lost(reason),
                };
                counters.add_rx(payload.len());
                last_server_frame = tokio::time::Instant::now();
                let msg: WireMessage = match wire::decode(&payload) {
                    Ok(msg) => msg,
                    Err(e) => {
                        tracing::warn!("undecodable message from server: {e}");
                        continue;
                    }
                };
                let WireMessage::Control(msg) = msg;
                tracing::trace!(
                    msg = msg.variant_name(),
                    bytes = payload.len(),
                    via_datagram,
                    "recv"
                );
                match msg {
                    ServerControl::AuthOk { observed_addr } => {
                        authenticated = true;
                        tracing::debug!(%observed_addr, "auth accepted");
                        let _ = events.send(NetworkEvent::Connected { observed_addr }).await;
                        // Open the dedicated relay stream for file
                        // transfer (the server accepts streams only
                        // post-auth). A failure here just leaves relay
                        // unavailable until the next connection.
                        match conn.open_stream().await {
                            Ok(stream) => {
                                let dessplay_core::net::BiStream { mut send, recv } = stream;
                                // Announce the stream immediately: QUIC
                                // reveals a bi-stream to the peer only on
                                // first write, so without this a peer
                                // that only receives never registers its
                                // relay stream and its messages are
                                // dropped server-side.
                                if let Ok(frame) = wire::encode(&RelayEnvelope::Hello)
                                    && let Err(e) = write_frame(&mut send, &frame).await
                                {
                                    tracing::warn!("announcing relay stream: {e}");
                                }
                                relay_send = Some(send);
                                tokio::spawn(run_relay_reader(
                                    recv,
                                    events.clone(),
                                    Arc::clone(&counters),
                                ));
                                tracing::debug!("relay stream opened");
                            }
                            Err(e) => tracing::warn!("opening relay stream: {e}"),
                        }
                    }
                    ServerControl::AuthFailed => {
                        return ConnectionEnd::Rejected("the server rejected the password".into());
                    }
                    ServerControl::ProtocolMismatch { server_version } => {
                        return ConnectionEnd::Rejected(format!(
                            "protocol version mismatch: the server speaks v{server_version}, \
                             this client v{} — please update dessplay",
                            config.protocol_version
                        ));
                    }
                    ServerControl::PeerList {
                        peers,
                        known_offline,
                    } => {
                        let _ = events
                            .send(NetworkEvent::PeerList {
                                peers,
                                known_offline,
                            })
                            .await;
                    }
                    ServerControl::TimeSyncResponse { client_send, server_recv, server_send } => {
                        let t4 = (config.clock)();
                        probe_outstanding = false;
                        probes_since_answer = 0;
                        timesync.add_exchange(client_send, server_recv, server_send, t4);
                        if let Some(offset) = timesync.offset()
                            && last_offset != Some(offset)
                        {
                            last_offset = Some(offset);
                            tracing::debug!(offset_millis = offset, "clock offset updated");
                            let _ = events
                                .send(NetworkEvent::ClockSync { offset_millis: offset })
                                .await;
                        }
                    }
                    ServerControl::AniDbSearchResults { query, results } => {
                        let _ = events
                            .send(NetworkEvent::SearchResults { query, results })
                            .await;
                    }
                    msg @ (ServerControl::StateOp { .. }
                    | ServerControl::StateMerge(_)
                    | ServerControl::StateSnapshot(_)
                    | ServerControl::StateHash { .. }) => {
                        let _ = events
                            .send(NetworkEvent::Server {
                                msg: Box::new(msg),
                                via_datagram,
                            })
                            .await;
                    }
                    other => {
                        // Terse: a Debug of `other` could dump a whole
                        // state snapshot.
                        tracing::debug!(
                            msg = other.variant_name(),
                            "ignoring unexpected server message"
                        );
                    }
                }
            }

            _ = tokio::time::sleep_until(next_probe), if authenticated => {
                // A probe still outstanding when the next one fires never
                // got its answer. Only counted at the steady-state cadence
                // (30s apart — an answer that slow is genuinely lost); the
                // 200ms burst probes overlap legitimately.
                if probe_outstanding && probes_sent > INITIAL_PROBE_BURST {
                    probes_since_answer += 1;
                }
                let probe = ServerControl::TimeSyncRequest { client_send: (config.clock)() };
                if let Err(e) = send_datagram_or_control(conn, &counters, &probe).await {
                    return ConnectionEnd::Lost(e.to_string());
                }
                probe_outstanding = true;
                probes_sent += 1;
                let delay = if probes_sent < INITIAL_PROBE_BURST {
                    BURST_INTERVAL
                } else {
                    config.time_sync_interval
                };
                next_probe = tokio::time::Instant::now() + delay;
            }

            _ = health_tick.tick(), if authenticated => {
                let now = tokio::time::Instant::now();
                let elapsed_millis = now.duration_since(last_health).as_millis().max(1) as u64;
                last_health = now;
                let tx = counters.tx_bytes.load(Ordering::Relaxed);
                let rx = counters.rx_bytes.load(Ordering::Relaxed);
                let report = LinkHealthReport {
                    rtt_millis: timesync.median_rtt(),
                    unanswered_probes: probes_since_answer,
                    server_silence_millis: last_server_frame.elapsed().as_millis() as u64,
                    tx_bps: (tx - last_tx) * 1000 / elapsed_millis,
                    rx_bps: (rx - last_rx) * 1000 / elapsed_millis,
                    quic: conn.link_stats(),
                };
                last_tx = tx;
                last_rx = rx;
                // Lossy by design: a health sample is superseded in a
                // second, and a full event channel must never stall the
                // actor's recv/send arms behind droppable metrics.
                let _ = events.try_send(NetworkEvent::LinkHealth(report));
            }

            cmd = commands.recv() => {
                match cmd {
                    Some(NetworkCommand::SendReliable(msg)) => {
                        if let Err(e) = send_control(conn, &counters, &msg).await {
                            return ConnectionEnd::Lost(e.to_string());
                        }
                    }
                    Some(NetworkCommand::SendEager(msg)) => {
                        if let Err(e) = send_eager(conn, &counters, &msg).await {
                            return ConnectionEnd::Lost(e.to_string());
                        }
                    }
                    Some(NetworkCommand::SendDatagramOnly(msg)) => {
                        // Best-effort by definition; errors are not lost
                        // data (the 1s reliable position tick catches up).
                        if let Ok(frame) =
                            wire::encode(&WireMessage::Control((*msg).clone()))
                        {
                            tracing::trace!(
                                msg = msg.variant_name(),
                                bytes = frame.len(),
                                "send datagram-only"
                            );
                            counters.add_tx(frame.len());
                            let _ = conn.send_datagram(&frame).await;
                        }
                    }
                    Some(NetworkCommand::SendPeer { to, message }) => {
                        let Some(send) = relay_send.as_mut() else {
                            tracing::debug!("relay stream not up; dropping SendPeer");
                            continue;
                        };
                        let Ok(inner) = wire::encode(&*message) else {
                            continue;
                        };
                        let envelope = RelayEnvelope::Forward { to, message: inner };
                        let Ok(frame) = wire::encode(&envelope) else {
                            continue;
                        };
                        counters.add_tx(frame.len());
                        // A relay write error almost always means the
                        // whole QUIC connection is dying; drop the relay
                        // half and let the recv() arm drive reconnect
                        // (which reopens the stream) rather than tearing
                        // down here on what might be a transient stream
                        // fault.
                        if let Err(e) = write_frame(send, &frame).await {
                            tracing::debug!("relay write failed: {e}");
                            relay_send = None;
                        }
                    }
                    Some(NetworkCommand::Shutdown) | None => {
                        // Graceful quit: tell the server so it removes
                        // us immediately (no Lost stage) and pauses
                        // playback. Wait for the server's close so the
                        // frame is actually flushed before we tear the
                        // connection down (closing first could discard
                        // it). Best-effort throughout — a failure just
                        // means we go through Lost like a crash.
                        if send_control(conn, &counters, &ServerControl::Goodbye).await.is_ok() {
                            let _ = tokio::time::timeout(Duration::from_millis(500), async {
                                loop {
                                    match conn.recv().await {
                                        Ok(TransportEvent::Closed { .. }) | Err(_) => break,
                                        Ok(_) => continue,
                                    }
                                }
                            })
                            .await;
                        }
                        return ConnectionEnd::Shutdown;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    use dessplay_core::net::Listener;
    use dessplay_core::net::sim::{EndpointId, LinkConfig, SimListener, SimNetwork, SimTransport};
    use dessplay_core::types::Ed2kHash;
    use tokio::sync::mpsc;

    use super::*;

    /// A connector whose connect() never resolves — a server that
    /// blackholes the handshake.
    struct NeverConnector;

    impl Connector for NeverConnector {
        type Conn = SimTransport;

        async fn connect(&self) -> Result<Self::Conn, TransportError> {
            std::future::pending().await
        }
    }

    fn config() -> NetworkConfig {
        NetworkConfig::new(
            UserId::new("kim"),
            "pw".into(),
            Role::Interactive,
            Arc::new(AtomicU64::new(0)),
            Arc::new(|| 0),
        )
    }

    /// Shutdown must interrupt a connection attempt — found by manual
    /// testing: Ctrl-C during startup left the process hanging on the
    /// network actor, which only read commands *between* attempts.
    #[tokio::test(start_paused = true)]
    async fn shutdown_interrupts_a_hung_connect() {
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let actor = tokio::spawn(run(
            Arc::new(NeverConnector),
            config(),
            command_rx,
            event_tx,
        ));

        command_tx.send(NetworkCommand::Shutdown).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), actor)
            .await
            .expect("network actor hung in connect() across a Shutdown")
            .unwrap();
    }

    /// A clock that follows paused tokio time from a fixed origin, so
    /// time-sync math sees real (virtual) durations.
    fn paused_clock() -> Clock {
        let origin = tokio::time::Instant::now();
        Arc::new(move || {
            1_700_000_000_000
                + tokio::time::Instant::now()
                    .duration_since(origin)
                    .as_millis() as u64
        })
    }

    fn config_with_clock(clock: Clock) -> NetworkConfig {
        NetworkConfig::new(
            UserId::new("kim"),
            "pw".into(),
            Role::Interactive,
            Arc::new(AtomicU64::new(0)),
            clock,
        )
    }

    /// A minimal server: accepts one connection, answers `Auth` with
    /// `AuthOk`, and — only when `answer_probes` — answers time-sync
    /// probes. With `answer_probes` false it authenticates and then
    /// goes silent while keeping the connection open: the sim has no
    /// idle timeout, so this is exactly the "QUIC alive, sync dead"
    /// saturated-uplink failure mode.
    async fn fake_server(listener: SimListener, clock: Clock, answer_probes: bool) {
        let Ok((conn, addr)) = listener.accept().await else {
            return;
        };
        loop {
            let payload = match conn.recv().await {
                Ok(TransportEvent::Control(bytes) | TransportEvent::Datagram(bytes)) => bytes,
                Ok(TransportEvent::IncomingStream(_)) => continue, // the relay stream
                Ok(TransportEvent::Closed { .. }) | Err(_) => return,
            };
            let Ok(WireMessage::Control(msg)) = wire::decode(&payload) else {
                continue;
            };
            let reply = match msg {
                ServerControl::Auth { .. } => ServerControl::AuthOk {
                    observed_addr: addr,
                },
                ServerControl::TimeSyncRequest { client_send } if answer_probes => {
                    let now = (clock)();
                    ServerControl::TimeSyncResponse {
                        client_send,
                        server_recv: now,
                        server_send: now,
                    }
                }
                _ => continue,
            };
            let frame = wire::encode(&WireMessage::Control(reply)).unwrap();
            let _ = conn.send_control(&frame).await;
        }
    }

    /// Spawn the actor against a fake server, run for `duration`, shut
    /// down, and return every LinkHealth report emitted.
    async fn collect_health(
        net: &SimNetwork,
        answer_probes: bool,
        duration: Duration,
        commands_during: impl FnOnce(mpsc::Sender<NetworkCommand>) -> mpsc::Sender<NetworkCommand>,
    ) -> Vec<LinkHealthReport> {
        let server = EndpointId::new("server");
        let listener = net.listener(&server);
        tokio::spawn(fake_server(listener, paused_clock(), answer_probes));

        let (command_tx, command_rx) = mpsc::channel(8);
        // Large buffer: the actor emits one report per second and the
        // test drains only at the end — a full channel would stall the
        // actor and deadlock paused time.
        let (event_tx, mut event_rx) = mpsc::channel(4096);
        let connector = Arc::new(net.connector(&EndpointId::new("kim"), &server));
        let actor = tokio::spawn(run(
            connector,
            config_with_clock(paused_clock()),
            command_rx,
            event_tx,
        ));

        let command_tx = commands_during(command_tx);
        tokio::time::sleep(duration).await;
        command_tx.send(NetworkCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), actor)
            .await
            .expect("clean shutdown")
            .unwrap();

        let mut reports = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            if let NetworkEvent::LinkHealth(report) = event {
                reports.push(report);
            }
        }
        reports
    }

    /// On a healthy 150ms-each-way link, the report carries the probe
    /// RTT (~300ms), a small server-silence age, and nonzero traffic
    /// during the probe burst.
    #[tokio::test(start_paused = true)]
    async fn link_health_reports_rtt_and_traffic() {
        let net = SimNetwork::new(7);
        net.set_default_link(LinkConfig {
            latency: Duration::from_millis(150),
            ..LinkConfig::default()
        });
        let reports = collect_health(&net, true, Duration::from_secs(5), |tx| tx).await;

        let last = reports.last().expect("health reports were emitted");
        let rtt = last.rtt_millis.expect("probes were answered");
        assert!(
            (290..=350).contains(&rtt),
            "median rtt should be ~300ms, got {rtt}"
        );
        assert!(
            last.server_silence_millis < 10_000,
            "server was talking; silence was {}ms",
            last.server_silence_millis
        );
        assert_eq!(last.unanswered_probes, 0);
        assert!(
            reports.iter().any(|r| r.tx_bps > 0 && r.rx_bps > 0),
            "the probe burst should register as traffic"
        );
    }

    /// The Starlink regression (2026-07-24): a saturated uplink kept
    /// the QUIC connection alive while sync silently died. A server
    /// that authenticates and then goes silent must show up as growing
    /// server silence and consecutive unanswered probes — while the
    /// connection is still up (the actor never saw a disconnect).
    #[tokio::test(start_paused = true)]
    async fn link_health_detects_silent_server_on_live_connection() {
        let net = SimNetwork::new(11);
        let reports = collect_health(&net, false, Duration::from_secs(160), |tx| tx).await;

        let last = reports.last().expect("health reports were emitted");
        assert!(
            last.server_silence_millis > 150_000,
            "server has been silent since AuthOk; reported {}ms",
            last.server_silence_millis
        );
        assert!(
            last.unanswered_probes >= 3,
            "steady-state probes went unanswered; reported {}",
            last.unanswered_probes
        );
        assert_eq!(last.rtt_millis, None, "no probe was ever answered");
    }

    /// Relay traffic (the file-transfer plane) counts toward the
    /// bandwidth numbers: a SendPeer chunk request shows up in tx_bps.
    #[tokio::test(start_paused = true)]
    async fn counters_cover_relay_bytes() {
        let net = SimNetwork::new(13);
        let reports = collect_health(&net, true, Duration::from_secs(4), |tx| {
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                // Wait past auth + relay-stream open, then push ~80KB
                // of chunk requests through the relay plane.
                tokio::time::sleep(Duration::from_secs(1)).await;
                let message = PeerMessage::ChunkRequest {
                    file: Ed2kHash([0x42; 16]),
                    chunks: (0..20_000).collect(),
                };
                let _ = tx_clone
                    .send(NetworkCommand::SendPeer {
                        to: UserId::new("nas"),
                        message: Box::new(message),
                    })
                    .await;
            });
            tx
        })
        .await;

        assert!(
            reports.iter().any(|r| r.tx_bps > 10_000),
            "an 80KB relay send should dominate some 1s window; max tx_bps was {:?}",
            reports.iter().map(|r| r.tx_bps).max()
        );
    }

    /// Sanity for the interrupt-capable connect path: a normal connect
    /// still works and still shuts down cleanly.
    #[tokio::test(start_paused = true)]
    async fn connect_still_succeeds_normally() {
        let net = SimNetwork::new(1);
        let server = EndpointId::new("server");
        let listener = net.listener(&server);
        tokio::spawn(async move {
            let accepted = listener.accept().await;
            // Hold the connection open until the test ends.
            if accepted.is_ok() {
                std::future::pending::<()>().await;
            }
        });
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let connector = Arc::new(net.connector(&EndpointId::new("kim"), &server));
        let actor = tokio::spawn(run(connector, config(), command_rx, event_tx));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        command_tx.send(NetworkCommand::Shutdown).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), actor)
            .await
            .expect("clean shutdown")
            .unwrap();
    }
}
