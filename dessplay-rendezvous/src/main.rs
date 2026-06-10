//! DessPlay rendezvous server binary. Phase 1 stub: the coordinator,
//! compaction, and AniDB integration arrive in later phases.

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    println!(
        "dessplay-rendezvous {} (phase 1 stub)",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}
