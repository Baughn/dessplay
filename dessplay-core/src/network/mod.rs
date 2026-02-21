//! Peer mesh network abstraction.
//!
//! The [`Network`] trait defines the interface for sending/receiving messages
//! between peers. Implementations include [`simulated::SimulatedNetwork`] for
//! testing and `QuicNetwork` (in the client crate) for production.

pub mod simulated;

use std::future::Future;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::protocol::{PeerControl, PeerDatagram};
use crate::types::PeerId;

/// Events produced by the network layer.
#[derive(Debug)]
pub enum NetworkEvent {
    /// A new peer has connected and completed the Hello handshake.
    PeerConnected {
        peer_id: PeerId,
        username: String,
    },
    /// A peer has disconnected.
    PeerDisconnected {
        peer_id: PeerId,
    },
    /// A reliable control message from a peer.
    PeerControl {
        from: PeerId,
        message: PeerControl,
    },
    /// An unreliable datagram from a peer.
    PeerDatagram {
        from: PeerId,
        message: PeerDatagram,
    },
    /// A peer opened a bidirectional stream to us.
    IncomingStream {
        from: PeerId,
        stream: MessageStream,
    },
}

/// A bidirectional message stream (wraps any AsyncRead/Write).
pub struct MessageStream {
    pub send: Box<dyn AsyncWrite + Send + Unpin>,
    pub recv: Box<dyn AsyncRead + Send + Unpin>,
}

impl std::fmt::Debug for MessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageStream").finish_non_exhaustive()
    }
}

/// The peer mesh network interface.
///
/// Handles peer-to-peer messaging via control messages (reliable), datagrams
/// (unreliable), and bidirectional streams (reliable, ordered).
pub trait Network: Send + Sync {
    /// Send a reliable control message to a specific peer.
    fn send_control(
        &self,
        peer: PeerId,
        msg: &PeerControl,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Send an unreliable datagram to a specific peer.
    fn send_datagram(
        &self,
        peer: PeerId,
        msg: &PeerDatagram,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Open a new bidirectional stream to a specific peer.
    fn open_stream(
        &self,
        peer: PeerId,
    ) -> impl Future<Output = anyhow::Result<MessageStream>> + Send;

    /// Receive the next network event.
    fn recv(&self) -> impl Future<Output = anyhow::Result<NetworkEvent>> + Send;

    /// List all currently connected peer IDs.
    fn connected_peers(&self) -> Vec<PeerId>;
}
