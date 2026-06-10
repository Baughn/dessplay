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
    loop {
        match connector.connect().await {
            Ok(conn) => match run_connection(&conn, &config, &mut commands, &events).await {
                ConnectionEnd::Shutdown => {
                    conn.close("shutting down").await;
                    return;
                }
                ConnectionEnd::AuthFailed => {
                    let _ = events.send(NetworkEvent::AuthFailed).await;
                    return;
                }
                ConnectionEnd::Lost(reason) => {
                    let _ = events.send(NetworkEvent::Disconnected { reason }).await;
                }
            },
            Err(e) => {
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
                match msg {
                    ServerControl::AuthOk { observed_addr } => {
                        authenticated = true;
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
                            let _ = events
                                .send(NetworkEvent::ClockSync { offset_millis: offset })
                                .await;
                        }
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
