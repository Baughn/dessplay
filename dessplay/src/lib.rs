//! DessPlay client library. The binary in `main.rs` is a thin shell —
//! everything constructible lives here so the multi-client simulation
//! harness can build complete clients (see architecture.md, Composition
//! Root).

#![deny(missing_docs)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]
#![deny(clippy::dbg_macro)]

pub mod actors;
pub mod client;
pub mod config;
pub mod storage;
