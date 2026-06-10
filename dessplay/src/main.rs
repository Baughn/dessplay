//! DessPlay client binary. Phase 2: storage and configuration exist;
//! actors, TUI, and player integration arrive in later phases.

#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]
#![deny(clippy::dbg_macro)]

mod config;
mod storage;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    println!("dessplay {} (phase 2 stub)", env!("CARGO_PKG_VERSION"));
    Ok(())
}
