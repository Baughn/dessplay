//! DessPlay rendezvous server library. The binary in `main.rs` is a
//! thin shell so tests (including cross-crate connection tests) can
//! construct and run the server in-process.

#![deny(missing_docs)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]
#![deny(clippy::dbg_macro)]

pub mod anidb;
pub mod server;
pub mod storage;
