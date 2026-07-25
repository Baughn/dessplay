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
pub mod advisor;
pub mod chunkstore;
pub mod client;
pub mod commentary;
pub mod config;
pub mod download;
pub mod dump;
pub mod import;
pub mod instance_lock;
pub mod logging;
pub mod placeholder;
pub mod player;
pub mod run;
pub mod seeder;
pub mod session;
pub mod storage;
pub mod timeutil;
pub mod torrent;
pub mod ui;
