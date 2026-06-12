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

use dessplay_core::net::timesync::TimeSync;
use dessplay_core::net::{
    Connector, Role, ServerControl, Transport, TransportError, TransportEvent, WireMessage,
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
            NetworkCommand::Shutdown => "Shutdown",
        }
    }
}

/// Events to the main loop.
#[derive(Debug)]
pub enum NetworkEvent {
    /// Authenticated; the server saw us at this address.
    Connected {
        /// Our address as observed by the server.
        observed_addr: SocketAddr,
    },
    /// The server rejected our password. Terminal — the actor exits.
    AuthFailed,
    /// A fresh peer list.
    PeerList(Vec<dessplay_core::net::PeerInfo>),
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
    /// Connection lost; the actor will retry.
    Disconnected {
        /// Human-readable cause.
        reason: String,
    },
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
        }
    }
}

/// Probes sent back-to-back right after connecting, to seed the offset
/// window before the steady-state cadence takes over.
const INITIAL_PROBE_BURST: u32 = 5;
/// Spacing of the initial burst.
const BURST_INTERVAL: Duration = Duration::from_millis(200);

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
                    ConnectionEnd::AuthFailed => {
                        tracing::warn!("server rejected our password");
                        let _ = events.send(NetworkEvent::AuthFailed).await;
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
    AuthFailed,
    Lost(String),
}

/// Send a control message, encoding it first.
async fn send_control<T: Transport>(conn: &T, msg: &ServerControl) -> Result<(), TransportError> {
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
    conn.send_control(&frame).await
}

/// Send a small message as a datagram, falling back to the control
/// stream when datagrams are unavailable.
async fn send_datagram_or_control<T: Transport>(
    conn: &T,
    msg: &ServerControl,
) -> Result<(), TransportError> {
    let frame = wire::encode(&WireMessage::Control(msg.clone()))
        .map_err(|e| TransportError::Setup(format!("encode: {e}")))?;
    tracing::trace!(
        msg = msg.variant_name(),
        bytes = frame.len(),
        "send datagram"
    );
    match conn.send_datagram(&frame).await {
        Err(TransportError::DatagramUnsupported | TransportError::DatagramTooLarge { .. }) => {
            conn.send_control(&frame).await
        }
        other => other,
    }
}

/// Send reliably, plus eagerly as a datagram when it fits (the size
/// rule): receivers dedup, so the datagram is pure latency win.
async fn send_eager<T: Transport>(conn: &T, msg: &ServerControl) -> Result<(), TransportError> {
    let frame = wire::encode(&WireMessage::Control(msg.clone()))
        .map_err(|e| TransportError::Setup(format!("encode: {e}")))?;
    tracing::trace!(msg = msg.variant_name(), bytes = frame.len(), "send eager");
    conn.send_control(&frame).await?;
    if conn
        .max_datagram_size()
        .is_some_and(|max| frame.len() <= max)
    {
        let _ = conn.send_datagram(&frame).await;
    }
    Ok(())
}

async fn run_connection<T: Transport>(
    conn: &T,
    config: &NetworkConfig,
    commands: &mut mpsc::Receiver<NetworkCommand>,
    events: &mpsc::Sender<NetworkEvent>,
) -> ConnectionEnd {
    // ---- Authenticate.
    let auth = ServerControl::Auth {
        username: config.username.clone(),
        password: config.password.clone(),
        role: config.role,
        epoch: Epoch(config.epoch.load(Ordering::SeqCst)),
    };
    if let Err(e) = send_control(conn, &auth).await {
        return ConnectionEnd::Lost(e.to_string());
    }
    tracing::debug!(user = %config.username.0, role = ?config.role, "auth sent");

    let mut timesync = TimeSync::new();
    let mut probes_sent: u32 = 0;
    let mut last_offset: Option<i64> = None;
    let mut authenticated = false;
    let mut next_probe = tokio::time::Instant::now(); // first probe right after AuthOk

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
                    }
                    ServerControl::AuthFailed => return ConnectionEnd::AuthFailed,
                    ServerControl::PeerList { peers } => {
                        let _ = events.send(NetworkEvent::PeerList(peers)).await;
                    }
                    ServerControl::TimeSyncResponse { client_send, server_recv, server_send } => {
                        let t4 = (config.clock)();
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
                        tracing::debug!("ignoring unexpected server message: {other:?}");
                    }
                }
            }

            _ = tokio::time::sleep_until(next_probe), if authenticated => {
                let probe = ServerControl::TimeSyncRequest { client_send: (config.clock)() };
                if let Err(e) = send_datagram_or_control(conn, &probe).await {
                    return ConnectionEnd::Lost(e.to_string());
                }
                probes_sent += 1;
                let delay = if probes_sent < INITIAL_PROBE_BURST {
                    BURST_INTERVAL
                } else {
                    config.time_sync_interval
                };
                next_probe = tokio::time::Instant::now() + delay;
            }

            cmd = commands.recv() => {
                match cmd {
                    Some(NetworkCommand::SendReliable(msg)) => {
                        if let Err(e) = send_control(conn, &msg).await {
                            return ConnectionEnd::Lost(e.to_string());
                        }
                    }
                    Some(NetworkCommand::SendEager(msg)) => {
                        if let Err(e) = send_eager(conn, &msg).await {
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
                            let _ = conn.send_datagram(&frame).await;
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
                        if send_control(conn, &ServerControl::Goodbye).await.is_ok() {
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
    use dessplay_core::net::sim::{EndpointId, SimNetwork, SimTransport};
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
