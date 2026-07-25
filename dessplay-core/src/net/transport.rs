//! The transport seam: one established client<->server connection,
//! plus connector/listener traits for establishing them.
//!
//! Implementations: QUIC via quinn ([`super::quic`]) in production, the
//! in-process [`super::sim`] (feature `test-support`) everywhere else.
//! The trait works in **frame bytes** — serialization happens above it
//! (so tests can measure and corrupt real encoded sizes), framing and
//! transmission below it.
//!
//! All methods take `&self`: implementations use interior mutability so
//! an actor can hold a `recv()` future in one `select!` arm while
//! sending from another. Exactly one task should call `recv()`.

use std::future::Future;
use std::net::SocketAddr;

use tokio::io::{AsyncRead, AsyncWrite};

use super::framing::FrameError;

/// Transport-layer errors.
#[derive(Debug)]
pub enum TransportError {
    /// The connection is gone (peer closed, timed out, or was killed).
    ConnectionLost(String),
    /// Framing violation on the control stream.
    Frame(FrameError),
    /// The path does not support datagrams.
    DatagramUnsupported,
    /// Datagram exceeds the path MTU; send it on the control stream
    /// instead (the "size rule").
    DatagramTooLarge {
        /// Encoded size of the rejected datagram.
        len: usize,
        /// Current maximum the path accepts.
        max: usize,
    },
    /// Connection establishment failed (dial, TLS, TOFU mismatch...).
    Setup(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::ConnectionLost(reason) => write!(f, "connection lost: {reason}"),
            TransportError::Frame(e) => write!(f, "framing error: {e}"),
            TransportError::DatagramUnsupported => write!(f, "datagrams unsupported on this path"),
            TransportError::DatagramTooLarge { len, max } => {
                write!(f, "datagram of {len} bytes exceeds path limit {max}")
            }
            TransportError::Setup(reason) => write!(f, "connection setup failed: {reason}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<FrameError> for TransportError {
    fn from(e: FrameError) -> Self {
        TransportError::Frame(e)
    }
}

/// A bidirectional byte stream (gap fill, file transfer). The framing
/// helpers in [`super::framing`] run on top of these halves.
pub struct BiStream {
    /// Write half.
    pub send: Box<dyn AsyncWrite + Send + Unpin>,
    /// Read half.
    pub recv: Box<dyn AsyncRead + Send + Unpin>,
}

impl std::fmt::Debug for BiStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BiStream")
    }
}

/// Point-in-time connection statistics, for transports that can
/// provide them (QUIC; the sim cannot). Supplementary display data
/// only: health classification must rely on the network actor's own
/// transport-agnostic measurements, so sim tests stay deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkStats {
    /// Current path round-trip estimate, milliseconds.
    pub rtt_millis: u64,
    /// Congestion events on the path since the connection opened.
    pub congestion_events: u64,
    /// Packets declared lost since the connection opened.
    pub lost_packets: u64,
}

/// Something the connection delivered.
#[derive(Debug)]
pub enum TransportEvent {
    /// A frame from the control stream (reliable, ordered).
    Control(Vec<u8>),
    /// A datagram (unreliable, unordered).
    Datagram(Vec<u8>),
    /// The peer opened a stream toward us.
    IncomingStream(BiStream),
    /// The connection ended. Terminal: `recv` returns only errors after.
    Closed {
        /// Human-readable cause.
        reason: String,
    },
}

/// One established connection.
pub trait Transport: Send + Sync + 'static {
    /// Send a frame on the control stream: reliable, ordered, prioritized
    /// above transfer streams.
    fn send_control(&self, frame: &[u8])
    -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Send a datagram: best-effort, unordered. Fails with
    /// [`TransportError::DatagramTooLarge`] rather than fragmenting —
    /// the caller falls back to the control stream.
    fn send_datagram(
        &self,
        frame: &[u8],
    ) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Current maximum datagram payload, if datagrams are supported.
    fn max_datagram_size(&self) -> Option<usize>;

    /// Open a bidirectional stream toward the peer.
    fn open_stream(&self) -> impl Future<Output = Result<BiStream, TransportError>> + Send;

    /// Receive the next event. Cancel-safe; intended for one reader task.
    fn recv(&self) -> impl Future<Output = Result<TransportEvent, TransportError>> + Send;

    /// Current connection statistics, when the transport tracks them.
    /// The default (and the sim) answers `None`.
    fn link_stats(&self) -> Option<LinkStats> {
        None
    }

    /// Close the connection with a reason the peer can observe.
    fn close(&self, reason: &str) -> impl Future<Output = ()> + Send;
}

/// Dials the server. Owned by the client's network actor so it can
/// reconnect.
pub trait Connector: Send + Sync + 'static {
    /// The connection type this connector produces.
    type Conn: Transport;

    /// Establish a connection (dial + TLS + TOFU).
    fn connect(&self) -> impl Future<Output = Result<Self::Conn, TransportError>> + Send;
}

/// Accepts client connections. Owned by the server.
pub trait Listener: Send + 'static {
    /// The connection type this listener produces.
    type Conn: Transport;

    /// Accept the next connection, returning it with the client's
    /// observed address.
    fn accept(
        &self,
    ) -> impl Future<Output = Result<(Self::Conn, SocketAddr), TransportError>> + Send;
}
