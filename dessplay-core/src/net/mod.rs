//! Network layer: wire messages, framing, the transport seam, time
//! sync, TOFU, and the QUIC and simulated transport implementations.
//!
//! See docs/network-design.md for the protocol this implements.

pub mod framing;
pub mod message;
pub mod quic;
#[cfg(feature = "test-support")]
pub mod sim;
pub mod timesync;
pub mod tofu;
pub mod transport;

pub use message::{AniDbSearchHit, PeerInfo, Presence, Role, ServerControl, WireMessage};
pub use transport::{BiStream, Connector, Listener, Transport, TransportError, TransportEvent};

/// The default rendezvous-server port, used when an address omits one.
pub const DEFAULT_PORT: u16 = 9876;
