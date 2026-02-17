pub mod clock;
pub mod quic;
pub mod rendezvous;
pub mod sync;
pub mod wire;

use async_trait::async_trait;
use tokio::sync::broadcast;

/// Opaque peer identifier. In production this wraps a QUIC connection ID
/// or similar; in tests it's an arbitrary string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct PeerId(pub String);

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Events emitted when peer connectivity changes.
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
    ConnectionStateChanged {
        peer: PeerId,
        state: ConnectionState,
    },
}

/// Per-peer connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Direct,
    Relayed,
}

/// Abstraction over peer-to-peer connectivity.
///
/// Provides two communication channels:
/// - **Datagrams**: unreliable, unordered, low-latency (gossip, clock sync)
/// - **Reliable**: ordered, guaranteed delivery (gap fill responses)
///
/// Implementations: `QuicConnectionManager` (production), `SimulatedConnectionManager` (tests).
#[async_trait]
pub trait ConnectionManager: Send + Sync {
    /// Send an unreliable datagram to a specific peer.
    /// May be silently dropped by the network.
    async fn send_datagram(&self, peer: &PeerId, data: &[u8]) -> Result<(), ConnectionError>;

    /// Receive the next incoming datagram from any peer.
    async fn recv_datagram(&self) -> Result<(PeerId, Vec<u8>), ConnectionError>;

    /// Send data reliably to a specific peer via a QUIC stream.
    /// Returns Err if the peer is disconnected or partitioned.
    async fn send_reliable(&self, peer: &PeerId, data: &[u8]) -> Result<(), ConnectionError>;

    /// Receive the next reliable message from any peer.
    async fn recv_reliable(&self) -> Result<(PeerId, Vec<u8>), ConnectionError>;

    /// Subscribe to connection events (connect, disconnect, state changes).
    fn subscribe(&self) -> broadcast::Receiver<ConnectionEvent>;

    /// List currently connected peers.
    fn connected_peers(&self) -> Vec<PeerId>;
}

/// Errors from connection operations.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("peer not connected: {0}")]
    PeerNotConnected(PeerId),

    #[error("network partitioned from peer: {0}")]
    Partitioned(PeerId),

    #[error("connection closed")]
    Closed,

    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}
