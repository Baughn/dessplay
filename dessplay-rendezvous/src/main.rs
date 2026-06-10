//! DessPlay rendezvous server binary. Phase 2: storage exists; the
//! coordinator, compaction, and AniDB integration arrive in later phases.

#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]
#![deny(clippy::dbg_macro)]

mod storage;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    println!(
        "dessplay-rendezvous {} (phase 2 stub)",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}
