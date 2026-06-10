//! DessPlay client binary. Phase 1 stub: actors, TUI, and player integration
//! arrive in later phases.

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    println!("dessplay {} (phase 1 stub)", env!("CARGO_PKG_VERSION"));
    Ok(())
}
