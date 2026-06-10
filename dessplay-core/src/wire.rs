//! Postcard serialization helpers for wire types ([`crate::state::CrdtOp`],
//! [`crate::state::StateSnapshot`], and anything else serde-derived).
//!
//! Framing (length prefixes on QUIC streams) is the network layer's job
//! and arrives in Phase 3; this module is just bytes <-> values.

use serde::{Deserialize, Serialize};

/// The codec error type, re-exported so downstream crates don't need a
/// direct postcard dependency.
pub use postcard::Error as WireError;

/// Serialize a wire value to postcard bytes.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(value)
}

/// Deserialize a wire value from postcard bytes.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}
