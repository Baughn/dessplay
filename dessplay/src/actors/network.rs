//! The network actor: owns the connections to the rendezvous server.
//!
//! Two QUIC connections since protocol v8 (the DSCP split — see
//! docs/proposals/2026-07-28-transfer-flow-control.md): the **control**
//! connection carries auth, state sync, peer lists, time sync, and
//! datagrams; the **transfer** connection carries the relay (file
//! transfer) stream. They are separate so each can ride a differently
//! DSCP-tagged socket. Presence is keyed to the control connection
//! alone: the transfer link redials on its own backoff and its death
//! degrades transfers, never liveness.
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
    /// server on the relay stream of the dedicated **transfer
    /// connection**. Dropped if the transfer link isn't up yet
    /// (pre-auth, or redialing) — transfer logic retries.
    SendPeer {
        /// The destination peer.
        to: PeerId,
        /// The message.
        message: Box<PeerMessage>,
    },
    /// Open a per-transfer **data stream** to `to` for `file` (the
    /// downloader side of a transfer). The server pumps it byte-for-byte
    /// to the target; the opened stream comes back as
    /// [`NetworkEvent::TransferStream`] with `outbound: true`.
    ///
    /// **Answered-request contract:** every open is answered — with the
    /// stream, or with [`NetworkEvent::TransferStreamFailed`] — never
    /// silently dropped. Requests arriving before the transfer link is
    /// up (the reconnect-until-AuthOk window) are buffered and drained
    /// on AuthOk. The file actor keys its "already asked" latch on this
    /// contract; a lost answer would wedge that transfer until restart.
    OpenTransferStream {
        /// The uploader.
        to: PeerId,
        /// The file to transfer.
        file: dessplay_core::types::Ed2kHash,
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
            NetworkCommand::OpenTransferStream { .. } => "OpenTransferStream",
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
    /// A per-transfer data stream is live: either the one we asked for
    /// ([`NetworkCommand::OpenTransferStream`], `outbound: true`) or one
    /// a peer opened toward us to download `file` (`outbound: false` —
    /// a serve request). The stream carries bare length-prefixed
    /// `PeerMessage` frames; the file actor owns it from here.
    TransferStream {
        /// The peer at the other end of the pump.
        peer: PeerId,
        /// The file this stream transfers.
        file: dessplay_core::types::Ed2kHash,
        /// Whether we opened it (our download) or they did (our serve).
        outbound: bool,
        /// The live stream.
        stream: dessplay_core::net::BiStream,
    },
    /// An [`NetworkCommand::OpenTransferStream`] could not be satisfied
    /// (link down or backlogged, the open itself failed, or the header
    /// write died). The answered-request contract's failure half: the
    /// file actor clears its pending queue for `(peer, file)` and
    /// re-requests on its own tick, so a failed open delays a transfer
    /// instead of wedging it.
    TransferStreamFailed {
        /// The uploader the stream was for.
        peer: PeerId,
        /// The file the stream was for.
        file: dessplay_core::types::Ed2kHash,
    },
    /// The transfer link has failed several consecutive dial/setup
    /// attempts. The control connection can be perfectly healthy while
    /// the transfer port (control + 1) is blocked — auth succeeds, chat
    /// flows, and every download silently sits at 0% — so past a small
    /// threshold the session is told, and the advisor turns it into
    /// the "is the transfer port open?" suggestion. The first few
    /// failures stay silent (transient blips self-heal on the backoff).
    TransferLinkDown {
        /// How many attempts in a row have failed.
        consecutive_failures: u32,
    },
    /// The transfer link came up after a [`NetworkEvent::TransferLinkDown`]
    /// was reported; clears the advisory.
    TransferLinkUp,
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
/// stream, datagrams, the relay stream, and every per-transfer data
/// stream in both directions (data streams are handed to the file
/// actor wrapped in [`CountedRead`]/[`CountedWrite`], so bulk chunk
/// traffic lands here even though this actor never sees the bytes —
/// the 2026-07-28 regression was a 100 Mb/s download showing ▲/▼ 0).
/// Shared with the spawned reader/stream tasks, hence atomics; created
/// fresh per connection so rate deltas reset naturally on reconnect.
#[derive(Debug, Default)]
struct NetCounters {
    tx_bytes: AtomicU64,
    rx_bytes: AtomicU64,
}

/// A write half that adds every written byte to the connection's tx
/// counter on its way through.
struct CountedWrite {
    inner: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    counters: Arc<NetCounters>,
}

impl tokio::io::AsyncWrite for CountedWrite {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let poll = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(written)) = &poll {
            self.counters.add_tx(*written);
        }
        poll
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A read half that adds every delivered byte to the connection's rx
/// counter on its way through.
struct CountedRead {
    inner: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    counters: Arc<NetCounters>,
}

impl tokio::io::AsyncRead for CountedRead {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &poll {
            self.counters.add_rx(buf.filled().len() - before);
        }
        poll
    }
}

/// Wrap both halves of a data stream in byte counters before handing it
/// out of the network layer.
fn counted_stream(
    stream: dessplay_core::net::BiStream,
    counters: &Arc<NetCounters>,
) -> dessplay_core::net::BiStream {
    let dessplay_core::net::BiStream { send, recv } = stream;
    dessplay_core::net::BiStream {
        send: Box::new(CountedWrite {
            inner: send,
            counters: Arc::clone(counters),
        }),
        recv: Box::new(CountedRead {
            inner: recv,
            counters: Arc::clone(counters),
        }),
    }
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

/// Hand a stream-open request to the transfer link, answering with
/// [`NetworkEvent::TransferStreamFailed`] when the link's channel is
/// full — the answered-request contract: every open gets the stream or
/// an explicit failure, so the file actor retries on its own tick
/// instead of waiting on an answer that will never come.
async fn forward_open(
    link: &TransferLink,
    to: PeerId,
    file: dessplay_core::types::Ed2kHash,
    events: &mpsc::Sender<NetworkEvent>,
) {
    if link
        .tx
        .try_send(TransferOp::OpenStream(to.clone(), file))
        .is_err()
    {
        tracing::debug!(%to, %file, "transfer link backlogged; answering open with failure");
        let _ = events
            .send(NetworkEvent::TransferStreamFailed { peer: to, file })
            .await;
    }
}

/// How many stream-open requests buffer while the transfer link is not
/// up yet (pre-AuthOk). Matches the link op channel's depth; past it
/// the newest request is answered with failure instead.
const PENDING_OPEN_BUFFER: usize = 64;

/// Consecutive failed transfer-link dial/setup attempts before the
/// session is told ([`NetworkEvent::TransferLinkDown`]). The first few
/// stay silent: transient blips self-heal on the backoff, but a
/// *blocked* transfer port (control + 1 must be opened separately)
/// fails every attempt forever while everything else looks healthy.
const TRANSFER_LINK_DOWN_THRESHOLD: u32 = 3;

/// Probes sent back-to-back right after connecting, to seed the offset
/// window before the steady-state cadence takes over.
const INITIAL_PROBE_BURST: u32 = 5;
/// Spacing of the initial burst.
const BURST_INTERVAL: Duration = Duration::from_millis(200);
/// Cadence of [`NetworkEvent::LinkHealth`] samples while authenticated.
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);

/// Run the network actor until shutdown or auth failure.
///
/// `transfer_connector` dials the transfer connection (control port + 1
/// in production, with the bulk DSCP tag); it is dialed after each
/// successful auth, bound to the session by the `AuthOk` token.
pub async fn run<C: Connector>(
    connector: Arc<C>,
    transfer_connector: Arc<C>,
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
                match run_connection(&conn, &transfer_connector, &config, &mut commands, &events)
                    .await
                {
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

/// Aborts a spawned task when dropped — ties the transfer link's
/// lifetime to the control connection that authorized it.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Work handed to the transfer link by the control loop.
enum TransferOp {
    /// Write a `Forward` envelope on the relay stream.
    Send(PeerId, Box<PeerMessage>),
    /// Open a data stream to a peer for a file (downloader side).
    OpenStream(PeerId, dessplay_core::types::Ed2kHash),
}

/// The control side's handle to the transfer link: outbound work goes
/// into `tx`; dropping the handle kills the task.
struct TransferLink {
    tx: mpsc::Sender<TransferOp>,
    _task: AbortOnDrop,
}

/// Own the transfer connection: dial, bind with `TransferAuth`, open
/// the relay stream, then pump outbound envelopes and surface inbound
/// `Forwarded` messages until the link dies — and redial. The loop is
/// deliberately self-healing and silent: a dead transfer link degrades
/// transfers (which retry at their own layer), never presence, so the
/// only observers are the log and the health line's byte counters.
async fn run_transfer_link<C: Connector>(
    connector: Arc<C>,
    username: UserId,
    token: u64,
    backoff: Duration,
    mut outbound: mpsc::Receiver<TransferOp>,
    events: mpsc::Sender<NetworkEvent>,
    counters: Arc<NetCounters>,
) {
    // Consecutive failed attempts to bring the link up (dial or
    // setup). A blocked transfer port fails every attempt forever
    // while everything else looks healthy, so past the threshold the
    // session is told once — and told again when the link recovers.
    let mut consecutive_failures: u32 = 0;
    let mut down_reported = false;
    loop {
        let conn = match connector.connect().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!("transfer link dial failed: {e}");
                note_transfer_link_failure(&events, &mut consecutive_failures, &mut down_reported)
                    .await;
                tokio::time::sleep(backoff).await;
                continue;
            }
        };
        let conn = Arc::new(conn);
        let link = async {
            let auth = ServerControl::TransferAuth {
                username: username.clone(),
                token,
            };
            send_control(&*conn, &counters, &auth).await?;
            let stream = conn.open_stream().await?;
            let dessplay_core::net::BiStream { mut send, recv } = stream;
            // Announce the stream immediately: QUIC reveals a bi-stream
            // to the peer only on first write, so without this a peer
            // that only receives never registers its relay stream on
            // the server and its inbound messages are dropped.
            let frame = wire::encode(&RelayEnvelope::Hello)
                .map_err(|e| TransportError::Setup(format!("encode: {e}")))?;
            write_frame(&mut send, &frame)
                .await
                .map_err(TransportError::from)?;
            Ok::<_, TransportError>((send, recv))
        };
        let (mut send, recv) = match link.await {
            Ok(halves) => halves,
            Err(e) => {
                tracing::debug!("transfer link setup failed: {e}");
                note_transfer_link_failure(&events, &mut consecutive_failures, &mut down_reported)
                    .await;
                conn.close("transfer link setup failed").await;
                tokio::time::sleep(backoff).await;
                continue;
            }
        };
        tracing::debug!("transfer link up");
        consecutive_failures = 0;
        if down_reported {
            down_reported = false;
            let _ = events.send(NetworkEvent::TransferLinkUp).await;
        }
        // The relay-stream reader runs apart from the writer (read_frame
        // is not cancel-safe, so it must not race in a select); its exit
        // is one link-death signal, `conn.recv()` errors the other.
        let mut reader = tokio::spawn(run_relay_reader(
            recv,
            events.clone(),
            Arc::clone(&counters),
        ));
        let dead = loop {
            tokio::select! {
                out = outbound.recv() => {
                    let Some(op) = out else {
                        // Actor gone: we die with it.
                        reader.abort();
                        conn.close("shutting down").await;
                        return;
                    };
                    match op {
                        TransferOp::Send(to, message) => {
                            let Ok(inner) = wire::encode(&*message) else { continue };
                            let envelope = RelayEnvelope::Forward { to, message: inner };
                            let Ok(frame) = wire::encode(&envelope) else { continue };
                            counters.add_tx(frame.len());
                            if let Err(e) = write_frame(&mut send, &frame).await {
                                tracing::debug!("transfer link write failed: {e}");
                                break false;
                            }
                        }
                        TransferOp::OpenStream(to, file) => {
                            // Open + header write + hand-off run in a
                            // task: at the transport's concurrent-stream
                            // cap `open_stream` *waits* for a credit
                            // rather than failing, and the header write
                            // can block on flow control — an await
                            // parked here would stop the loop accepting
                            // server-pumped serve streams and drain the
                            // op channel into try_send drops. A failure
                            // inside the task is answered with
                            // TransferStreamFailed (never silence); a
                            // dead connection is observed by the
                            // conn.recv() arm as usual.
                            tokio::spawn(open_data_stream(
                                Arc::clone(&conn),
                                to,
                                file,
                                events.clone(),
                                Arc::clone(&counters),
                            ));
                        }
                    }
                }
                // The server opens pump streams toward us when a peer
                // downloads from us; recv() also observes link death.
                event = conn.recv() => match event {
                    Ok(TransportEvent::IncomingStream(stream)) => {
                        tokio::spawn(classify_incoming_stream(
                            counted_stream(stream, &counters),
                            events.clone(),
                        ));
                    }
                    Ok(TransportEvent::Closed { .. }) | Err(_) => break true,
                    Ok(_) => {} // stray control frames / datagrams
                },
                _ = &mut reader => break true,
            }
        };
        if !dead {
            reader.abort();
        }
        conn.close("transfer link reset").await;
        tokio::time::sleep(backoff).await;
    }
}

/// One more failed attempt to bring the transfer link up: at the
/// threshold, tell the session once (see
/// [`NetworkEvent::TransferLinkDown`]); further failures stay quiet
/// until the link recovers.
async fn note_transfer_link_failure(
    events: &mpsc::Sender<NetworkEvent>,
    consecutive_failures: &mut u32,
    down_reported: &mut bool,
) {
    *consecutive_failures += 1;
    if *consecutive_failures == TRANSFER_LINK_DOWN_THRESHOLD && !*down_reported {
        *down_reported = true;
        tracing::warn!(
            consecutive_failures = *consecutive_failures,
            "transfer link is not coming up (is the transfer port open?)"
        );
        let _ = events
            .send(NetworkEvent::TransferLinkDown {
                consecutive_failures: *consecutive_failures,
            })
            .await;
    }
}

/// Open a data stream toward `to` for `file`, write its `OpenTransfer`
/// header, and surface it as an outbound [`NetworkEvent::TransferStream`].
/// Runs as its own task — the open can wait indefinitely for a stream
/// credit and the header write can block on flow control, neither of
/// which may park the link loop. Every failure is answered with
/// [`NetworkEvent::TransferStreamFailed`] (the answered-request
/// contract): the file actor clears its pending queue and retries on
/// its own tick, so a failed open can never wedge a transfer.
async fn open_data_stream<T: Transport>(
    conn: Arc<T>,
    to: PeerId,
    file: dessplay_core::types::Ed2kHash,
    events: mpsc::Sender<NetworkEvent>,
    counters: Arc<NetCounters>,
) {
    let opened = async {
        let stream = conn.open_stream().await?;
        let dessplay_core::net::BiStream { mut send, recv } = counted_stream(stream, &counters);
        let header = RelayEnvelope::OpenTransfer {
            to: to.clone(),
            file,
        };
        let frame =
            wire::encode(&header).map_err(|e| TransportError::Setup(format!("encode: {e}")))?;
        write_frame(&mut send, &frame)
            .await
            .map_err(TransportError::from)?;
        Ok::<_, TransportError>(dessplay_core::net::BiStream { send, recv })
    };
    match opened.await {
        Ok(stream) => {
            let _ = events
                .send(NetworkEvent::TransferStream {
                    peer: to,
                    file,
                    outbound: true,
                    stream,
                })
                .await;
        }
        Err(e) => {
            tracing::debug!(%to, %file, "opening data stream failed: {e}");
            let _ = events
                .send(NetworkEvent::TransferStreamFailed { peer: to, file })
                .await;
        }
    }
}

/// Read an incoming (server-pumped) stream's header and surface it as an
/// inbound [`NetworkEvent::TransferStream`] — a peer wants to download
/// `file` from us. Anything else is dropped.
async fn classify_incoming_stream(
    stream: dessplay_core::net::BiStream,
    events: mpsc::Sender<NetworkEvent>,
) {
    let dessplay_core::net::BiStream { send, mut recv } = stream;
    let header =
        tokio::time::timeout(std::time::Duration::from_secs(10), read_frame(&mut recv)).await;
    let Ok(Ok(frame)) = header else {
        tracing::debug!("incoming stream closed or silent before classifying; dropping");
        return;
    };
    match wire::decode::<RelayEnvelope>(&frame) {
        Ok(RelayEnvelope::TransferFrom { from, file }) => {
            let _ = events
                .send(NetworkEvent::TransferStream {
                    peer: from,
                    file,
                    outbound: false,
                    stream: dessplay_core::net::BiStream { send, recv },
                })
                .await;
        }
        Ok(other) => {
            tracing::warn!(header = ?other, "incoming stream with a non-header envelope");
        }
        Err(e) => tracing::warn!("undecodable incoming stream header: {e}"),
    }
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
            // The server only ever sends Forwarded on the relay stream
            // (data-stream headers arrive on their own streams).
            Ok(other) => {
                tracing::debug!(envelope = ?other, "unexpected relay envelope; ignoring");
            }
            Err(e) => tracing::warn!("undecodable relay envelope: {e}"),
        }
    }
    tracing::trace!("relay reader exiting");
}

async fn run_connection<T: Transport, C: Connector>(
    conn: &T,
    transfer_connector: &Arc<C>,
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
    // The transfer link, spawned on AuthOk with that auth's token. Its
    // task dies with this connection (AbortOnDrop): a reconnect gets a
    // fresh token and a fresh link.
    let mut transfer: Option<TransferLink> = None;
    // Stream-open requests that arrived before the transfer link exists
    // (the reconnect-until-AuthOk window). Buffered, not dropped — the
    // answered-request contract — and drained into the link on AuthOk.
    let mut pending_opens: Vec<(PeerId, dessplay_core::types::Ed2kHash)> = Vec::new();

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
                    ServerControl::AuthOk { observed_addr, transfer_token } => {
                        authenticated = true;
                        tracing::debug!(%observed_addr, "auth accepted");
                        let _ = events.send(NetworkEvent::Connected { observed_addr }).await;
                        // Bring up the transfer connection with this
                        // auth's token. The link redials itself; a
                        // failure only degrades transfers (which retry),
                        // never this connection.
                        let (tx, rx) = mpsc::channel(64);
                        let task = tokio::spawn(run_transfer_link(
                            Arc::clone(transfer_connector),
                            config.username.clone(),
                            transfer_token,
                            config.reconnect_backoff,
                            rx,
                            events.clone(),
                            Arc::clone(&counters),
                        ));
                        let link = TransferLink {
                            tx,
                            _task: AbortOnDrop(task),
                        };
                        // Answer the opens that buffered while the link
                        // was coming up.
                        for (to, file) in pending_opens.drain(..) {
                            forward_open(&link, to, file, events).await;
                        }
                        transfer = Some(link);
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
                        // Hand off to the transfer link. try_send, not
                        // send: a backlogged link must never stall the
                        // control loop, and a dropped peer message is
                        // recoverable by design (transfer logic retries).
                        let Some(link) = transfer.as_ref() else {
                            tracing::debug!("transfer link not up; dropping SendPeer");
                            continue;
                        };
                        if let Err(e) = link.tx.try_send(TransferOp::Send(to, message)) {
                            tracing::debug!("transfer link backlogged; dropping SendPeer: {e}");
                        }
                    }
                    Some(NetworkCommand::OpenTransferStream { to, file }) => {
                        // Answered-request contract: the stream, an
                        // explicit failure, or (pre-AuthOk) a buffered
                        // request — never a silent drop.
                        match transfer.as_ref() {
                            Some(link) => forward_open(link, to, file, events).await,
                            None if pending_opens.len() < PENDING_OPEN_BUFFER => {
                                pending_opens.push((to, file));
                            }
                            None => {
                                tracing::debug!(
                                    %to, %file,
                                    "pre-auth open buffer full; answering with failure"
                                );
                                let _ = events
                                    .send(NetworkEvent::TransferStreamFailed { peer: to, file })
                                    .await;
                            }
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
                    transfer_token: 42,
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

    /// A minimal transfer-side server: accepts any number of transfer
    /// connections, ignores their `TransferAuth`, drains relay streams
    /// (`Hello` first frame), and **echoes** every frame of any other
    /// stream — a data stream's bytes come straight back, so tests can
    /// drive both counter directions without a real peer.
    async fn fake_transfer_server(listener: SimListener) {
        loop {
            let Ok((conn, _addr)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                loop {
                    match conn.recv().await {
                        Ok(TransportEvent::IncomingStream(stream)) => {
                            let dessplay_core::net::BiStream { mut send, mut recv } = stream;
                            tokio::spawn(async move {
                                let mut first = true;
                                let mut echo = false;
                                while let Ok(frame) =
                                    dessplay_core::net::framing::read_frame(&mut recv).await
                                {
                                    if first {
                                        first = false;
                                        echo = !matches!(
                                            wire::decode::<RelayEnvelope>(&frame),
                                            Ok(RelayEnvelope::Hello)
                                        );
                                    }
                                    if echo
                                        && dessplay_core::net::framing::write_frame(
                                            &mut send, &frame,
                                        )
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                            });
                        }
                        Ok(TransportEvent::Closed { .. }) | Err(_) => return,
                        Ok(_) => continue, // TransferAuth, datagrams
                    }
                }
            });
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
        let transfer = EndpointId::new("server-transfer");
        tokio::spawn(fake_transfer_server(net.listener(&transfer)));

        let (command_tx, command_rx) = mpsc::channel(8);
        // Large buffer: the actor emits one report per second and the
        // test drains only at the end — a full channel would stall the
        // actor and deadlock paused time.
        let (event_tx, mut event_rx) = mpsc::channel(4096);
        let connector = Arc::new(net.connector(&EndpointId::new("kim"), &server));
        let transfer_connector = Arc::new(net.connector(&EndpointId::new("kim"), &transfer));
        let actor = tokio::spawn(run(
            connector,
            transfer_connector,
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

    /// Regression (2026-07-28): a ~100 Mb/s download showed ▲/▼ 0 in the
    /// health line — per-transfer data streams are owned by the file
    /// actor, so their bytes bypassed the network actor's counters until
    /// the halves were wrapped in counting adapters at hand-off. Pump
    /// bytes through a data stream in both directions and require them
    /// in a LinkHealth sample's rates.
    #[tokio::test(start_paused = true)]
    async fn counters_cover_data_stream_bytes() {
        use dessplay_core::net::framing::{read_frame, write_frame};

        let net = SimNetwork::new(17);
        let server = EndpointId::new("server");
        tokio::spawn(fake_server(net.listener(&server), paused_clock(), true));
        let transfer = EndpointId::new("server-transfer");
        tokio::spawn(fake_transfer_server(net.listener(&transfer)));

        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(4096);
        let connector = Arc::new(net.connector(&EndpointId::new("kim"), &server));
        let transfer_connector = Arc::new(net.connector(&EndpointId::new("kim"), &transfer));
        let _actor = tokio::spawn(run(
            connector,
            transfer_connector,
            config_with_clock(paused_clock()),
            command_rx,
            event_tx,
        ));

        // Once connected, ask for a data stream; the fake server echoes
        // its frames, so writing through it drives both directions.
        let mut stream = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match event_rx.recv().await.expect("actor alive") {
                    NetworkEvent::Connected { .. } => {
                        command_tx
                            .send(NetworkCommand::OpenTransferStream {
                                to: UserId::new("nas"),
                                file: Ed2kHash([1; 16]),
                            })
                            .await
                            .unwrap();
                    }
                    NetworkEvent::TransferStream {
                        outbound: true,
                        stream,
                        ..
                    } => break stream,
                    _ => {}
                }
            }
        })
        .await
        .expect("data stream never arrived");

        // What the file actor would do: write a bulk frame, read replies.
        let payload = vec![0xAB; 64 * 1024];
        write_frame(&mut stream.send, &payload).await.unwrap();
        let _header_echo = read_frame(&mut stream.recv).await.unwrap();
        let echoed = read_frame(&mut stream.recv).await.unwrap();
        assert_eq!(echoed, payload, "the fake server echoes data frames");

        // The next health sample must carry both directions' bytes.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let NetworkEvent::LinkHealth(report) =
                    event_rx.recv().await.expect("actor alive")
                    && report.tx_bps > 10_000
                    && report.rx_bps > 10_000
                {
                    break;
                }
            }
        })
        .await
        .expect("data-stream bytes never showed up in the health rates");
    }

    /// A sim-backed transport whose `open_stream` hangs once a budget of
    /// allowed opens is spent — models quinn parked awaiting a stream
    /// credit at `max_concurrent_bidi_streams`.
    struct BudgetedTransport {
        inner: SimTransport,
        open_budget: Arc<AtomicU32>,
    }

    impl Transport for BudgetedTransport {
        async fn send_control(&self, frame: &[u8]) -> Result<(), TransportError> {
            self.inner.send_control(frame).await
        }

        async fn send_datagram(&self, frame: &[u8]) -> Result<(), TransportError> {
            self.inner.send_datagram(frame).await
        }

        fn max_datagram_size(&self) -> Option<usize> {
            self.inner.max_datagram_size()
        }

        async fn open_stream(&self) -> Result<dessplay_core::net::BiStream, TransportError> {
            let mut budget = self.open_budget.load(Ordering::SeqCst);
            loop {
                if budget == 0 {
                    // Out of credits: wait forever, exactly like
                    // quinn's open_bi at the concurrency cap.
                    std::future::pending::<()>().await;
                }
                match self.open_budget.compare_exchange(
                    budget,
                    budget - 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => budget = actual,
                }
            }
            self.inner.open_stream().await
        }

        async fn recv(&self) -> Result<TransportEvent, TransportError> {
            self.inner.recv().await
        }

        async fn close(&self, reason: &str) {
            self.inner.close(reason).await
        }
    }

    /// Connector wrapper for [`BudgetedTransport`], optionally refusing
    /// to connect at all (a blocked transfer port).
    struct BudgetedConnector {
        inner: dessplay_core::net::sim::SimConnector,
        open_budget: Arc<AtomicU32>,
        never_connect: bool,
    }

    impl BudgetedConnector {
        fn new(inner: dessplay_core::net::sim::SimConnector, open_budget: u32) -> Self {
            BudgetedConnector {
                inner,
                open_budget: Arc::new(AtomicU32::new(open_budget)),
                never_connect: false,
            }
        }
    }

    impl Connector for BudgetedConnector {
        type Conn = BudgetedTransport;

        async fn connect(&self) -> Result<Self::Conn, TransportError> {
            if self.never_connect {
                std::future::pending::<()>().await;
            }
            let inner = self.inner.connect().await?;
            Ok(BudgetedTransport {
                inner,
                open_budget: Arc::clone(&self.open_budget),
            })
        }
    }

    use std::sync::atomic::{AtomicU32, Ordering};

    /// The answered-request contract, buffering half: an
    /// `OpenTransferStream` arriving before AuthOk (the window where
    /// the transfer link does not exist yet) must be buffered and
    /// satisfied once the link comes up — pre-fix it was silently
    /// dropped, permanently wedging that (source, file) transfer.
    #[tokio::test(start_paused = true)]
    async fn an_open_requested_before_auth_ok_is_buffered_and_answered() {
        let net = SimNetwork::new(23);
        let server = EndpointId::new("server");
        let listener = net.listener(&server);
        let clock = paused_clock();
        // A control server that answers Auth only after 500ms — the
        // pre-AuthOk window the buffering exists for.
        tokio::spawn(async move {
            let Ok((conn, addr)) = listener.accept().await else {
                return;
            };
            loop {
                let payload = match conn.recv().await {
                    Ok(TransportEvent::Control(bytes) | TransportEvent::Datagram(bytes)) => bytes,
                    Ok(TransportEvent::IncomingStream(_)) => continue,
                    Ok(TransportEvent::Closed { .. }) | Err(_) => return,
                };
                let Ok(WireMessage::Control(msg)) = wire::decode(&payload) else {
                    continue;
                };
                let reply = match msg {
                    ServerControl::Auth { .. } => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        ServerControl::AuthOk {
                            observed_addr: addr,
                            transfer_token: 42,
                        }
                    }
                    ServerControl::TimeSyncRequest { client_send } => {
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
        });
        let transfer = EndpointId::new("server-transfer");
        tokio::spawn(fake_transfer_server(net.listener(&transfer)));

        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(4096);
        let connector = Arc::new(net.connector(&EndpointId::new("kim"), &server));
        let transfer_connector = Arc::new(net.connector(&EndpointId::new("kim"), &transfer));
        let _actor = tokio::spawn(run(
            connector,
            transfer_connector,
            config_with_clock(paused_clock()),
            command_rx,
            event_tx,
        ));

        // Let the connect + Auth happen, then request the stream while
        // AuthOk is still pending.
        tokio::time::sleep(Duration::from_millis(50)).await;
        command_tx
            .send(NetworkCommand::OpenTransferStream {
                to: UserId::new("nas"),
                file: Ed2kHash([1; 16]),
            })
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match event_rx.recv().await.expect("actor alive") {
                    NetworkEvent::TransferStream { outbound: true, .. } => break,
                    NetworkEvent::TransferStreamFailed { .. } => {
                        panic!("a buffered open must not be answered with failure")
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("an open requested before AuthOk was dropped instead of buffered");
    }

    /// The inline-open hazard (2026-08-12 review): at the transport's
    /// concurrent-stream cap, `open_stream` *waits* — parked inline in
    /// the link loop it would stop `conn.recv()` being polled, so
    /// server-pumped serve streams toward us would never be accepted.
    /// The open must run off-loop: while one open hangs forever, an
    /// incoming stream must still surface.
    #[tokio::test(start_paused = true)]
    async fn a_hung_stream_open_does_not_stall_the_transfer_link() {
        let net = SimNetwork::new(29);
        let server = EndpointId::new("server");
        tokio::spawn(fake_server(net.listener(&server), paused_clock(), true));
        let transfer = EndpointId::new("server-transfer");
        let transfer_listener = net.listener(&transfer);
        let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
        // A transfer server that, on signal, opens a TransferFrom
        // stream toward the client (a peer wants to download from us).
        tokio::spawn(async move {
            let Ok((conn, _addr)) = transfer_listener.accept().await else {
                return;
            };
            let conn = Arc::new(conn);
            let pusher = Arc::clone(&conn);
            tokio::spawn(async move {
                let _ = go_rx.await;
                let Ok(mut stream) = pusher.open_stream().await else {
                    return;
                };
                let header = wire::encode(&RelayEnvelope::TransferFrom {
                    from: UserId::new("nas"),
                    file: Ed2kHash([7; 16]),
                })
                .unwrap();
                let _ = write_frame(&mut stream.send, &header).await;
                // Keep the stream alive.
                std::future::pending::<()>().await
            });
            // Hold incoming streams (the client's relay stream) —
            // dropping one closes it and tears the link down.
            let mut held = Vec::new();
            loop {
                match conn.recv().await {
                    Ok(TransportEvent::IncomingStream(stream)) => held.push(stream),
                    Ok(TransportEvent::Closed { .. }) | Err(_) => return,
                    Ok(_) => continue,
                }
            }
        });

        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(4096);
        // Control opens are unlimited; the transfer connection has
        // exactly one credit — the relay stream — so the data-stream
        // open hangs forever.
        let connector = Arc::new(BudgetedConnector::new(
            net.connector(&EndpointId::new("kim"), &server),
            u32::MAX,
        ));
        let transfer_connector = Arc::new(BudgetedConnector::new(
            net.connector(&EndpointId::new("kim"), &transfer),
            1,
        ));
        let _actor = tokio::spawn(run(
            connector,
            transfer_connector,
            config_with_clock(paused_clock()),
            command_rx,
            event_tx,
        ));

        // Wait for auth + link-up, then request a data stream whose
        // open will hang at the (exhausted) stream budget.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let NetworkEvent::Connected { .. } = event_rx.recv().await.expect("actor alive")
                {
                    break;
                }
            }
        })
        .await
        .expect("never connected");
        tokio::time::sleep(Duration::from_millis(500)).await; // link-up
        command_tx
            .send(NetworkCommand::OpenTransferStream {
                to: UserId::new("nas"),
                file: Ed2kHash([1; 16]),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await; // reach the link loop

        // The server now pushes a serve stream toward us. It must
        // surface even though the outbound open is parked forever.
        go_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let NetworkEvent::TransferStream {
                    outbound: false, ..
                } = event_rx.recv().await.expect("actor alive")
                {
                    break;
                }
            }
        })
        .await
        .expect("a hung outbound open starved incoming stream acceptance");
    }

    /// The answered-request contract, failure half: opens the link
    /// cannot take (its op channel is full because the link task is
    /// stuck dialing) are answered with `TransferStreamFailed`, never
    /// silently dropped.
    #[tokio::test(start_paused = true)]
    async fn a_backlogged_transfer_link_answers_opens_with_failure() {
        let net = SimNetwork::new(31);
        let server = EndpointId::new("server");
        tokio::spawn(fake_server(net.listener(&server), paused_clock(), true));

        let (command_tx, command_rx) = mpsc::channel(256);
        let (event_tx, mut event_rx) = mpsc::channel(4096);
        let connector = Arc::new(BudgetedConnector::new(
            net.connector(&EndpointId::new("kim"), &server),
            u32::MAX,
        ));
        // The transfer dial never completes: ops pile into the link's
        // 64-slot channel.
        let transfer_connector = Arc::new(BudgetedConnector {
            inner: net.connector(&EndpointId::new("kim"), &EndpointId::new("server-transfer")),
            open_budget: Arc::new(AtomicU32::new(u32::MAX)),
            never_connect: true,
        });
        let _actor = tokio::spawn(run(
            connector,
            transfer_connector,
            config_with_clock(paused_clock()),
            command_rx,
            event_tx,
        ));

        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let NetworkEvent::Connected { .. } = event_rx.recv().await.expect("actor alive")
                {
                    break;
                }
            }
        })
        .await
        .expect("never connected");

        // 70 opens into a 64-slot channel: the overflow must each be
        // answered with an explicit failure.
        for i in 0..70u8 {
            command_tx
                .send(NetworkCommand::OpenTransferStream {
                    to: UserId::new("nas"),
                    file: Ed2kHash([i; 16]),
                })
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut failures = 0;
        while let Ok(event) = event_rx.try_recv() {
            if let NetworkEvent::TransferStreamFailed { .. } = event {
                failures += 1;
            }
        }
        assert_eq!(
            failures, 6,
            "every open past the link channel's capacity must be answered with failure"
        );
    }

    /// A connector that fails its first `fail_first` connect attempts,
    /// then delegates to the sim — a transfer port that starts blocked
    /// and later opens.
    struct FlakyConnector {
        inner: dessplay_core::net::sim::SimConnector,
        fail_remaining: Arc<AtomicU32>,
    }

    impl Connector for FlakyConnector {
        type Conn = SimTransport;

        async fn connect(&self) -> Result<Self::Conn, TransportError> {
            let mut remaining = self.fail_remaining.load(Ordering::SeqCst);
            loop {
                if remaining == 0 {
                    return self.inner.connect().await;
                }
                match self.fail_remaining.compare_exchange(
                    remaining,
                    remaining - 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => return Err(TransportError::Setup("port blocked".into())),
                    Err(actual) => remaining = actual,
                }
            }
        }
    }

    /// Spawn the actor with a control link that works and a transfer
    /// link that fails its first `fail_first` dials; run for `duration`
    /// and return the (Down, Up) transfer-link events observed.
    async fn transfer_link_events(
        seed: u64,
        fail_first: u32,
        duration: Duration,
    ) -> (Vec<u32>, u32) {
        let net = SimNetwork::new(seed);
        let server = EndpointId::new("server");
        tokio::spawn(fake_server(net.listener(&server), paused_clock(), true));
        let transfer = EndpointId::new("server-transfer");
        tokio::spawn(fake_transfer_server(net.listener(&transfer)));

        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(4096);
        let connector = Arc::new(FlakyConnector {
            inner: net.connector(&EndpointId::new("kim"), &server),
            fail_remaining: Arc::new(AtomicU32::new(0)),
        });
        let transfer_connector = Arc::new(FlakyConnector {
            inner: net.connector(&EndpointId::new("kim"), &transfer),
            fail_remaining: Arc::new(AtomicU32::new(fail_first)),
        });
        let actor = tokio::spawn(run(
            connector,
            transfer_connector,
            config_with_clock(paused_clock()),
            command_rx,
            event_tx,
        ));
        tokio::time::sleep(duration).await;
        command_tx.send(NetworkCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), actor)
            .await
            .expect("clean shutdown")
            .unwrap();

        let mut downs = Vec::new();
        let mut ups = 0;
        while let Ok(event) = event_rx.try_recv() {
            match event {
                NetworkEvent::TransferLinkDown {
                    consecutive_failures,
                } => downs.push(consecutive_failures),
                NetworkEvent::TransferLinkUp => ups += 1,
                _ => {}
            }
        }
        (downs, ups)
    }

    /// A blocked transfer port is invisible in every other signal (auth
    /// succeeds, chat flows) while downloads sit at 0% forever: past
    /// [`TRANSFER_LINK_DOWN_THRESHOLD`] consecutive dial failures the
    /// actor must say so — once, not once per failure — and report
    /// recovery when a later dial succeeds.
    #[tokio::test(start_paused = true)]
    async fn a_repeatedly_failing_transfer_dial_is_reported_past_the_threshold() {
        // Fails far past the threshold, then recovers (backoff is 2s;
        // 60s covers plenty of attempts).
        let (downs, ups) = transfer_link_events(37, 10, Duration::from_secs(60)).await;
        assert_eq!(
            downs,
            vec![TRANSFER_LINK_DOWN_THRESHOLD],
            "one Down at the threshold, no repeats while it stays down"
        );
        assert_eq!(ups, 1, "recovery is reported once the link comes up");
    }

    /// The first few dial failures stay silent — transient blips
    /// self-heal on the backoff and must not ping the advisor.
    #[tokio::test(start_paused = true)]
    async fn a_brief_transfer_dial_blip_stays_silent() {
        let below = TRANSFER_LINK_DOWN_THRESHOLD - 1;
        let (downs, ups) = transfer_link_events(41, below, Duration::from_secs(60)).await;
        assert!(
            downs.is_empty(),
            "failures below the threshold must stay silent: {downs:?}"
        );
        assert_eq!(ups, 0, "no Down means no Up");
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
        let transfer_connector =
            Arc::new(net.connector(&EndpointId::new("kim"), &EndpointId::new("server-transfer")));
        let actor = tokio::spawn(run(
            connector,
            transfer_connector,
            config(),
            command_rx,
            event_tx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        command_tx.send(NetworkCommand::Shutdown).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), actor)
            .await
            .expect("clean shutdown")
            .unwrap();
    }
}
