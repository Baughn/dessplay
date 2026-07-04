//! DessPlay core: shared types, CRDT state, wire serialization, and ed2k
//! hashing. Pure logic — no networking, no I/O beyond hashing readers.
//!
//! See docs/design.md and docs/sync-state.md for the design this
//! implements.

#![deny(missing_docs)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]
#![deny(clippy::dbg_macro)]

pub mod compact;
pub mod derive;
pub mod episode_parse;
pub mod franchise;
pub mod hash;
pub mod lww;
pub mod net;
pub mod playlist;
pub mod series_identity;
pub mod state;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod types;
pub mod wire;

pub use lww::{Lww, LwwCell, resolve, resolve_value};
pub use playlist::{NewPlaylistEntry, PlaylistEntry};
pub use state::{CrdtOp, CrdtState, LwwMap, StateSnapshot, StateView};
pub use types::*;
