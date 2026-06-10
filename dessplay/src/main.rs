//! DessPlay client binary — a thin shell over the library (see
//! architecture.md, Composition Root). Real CLI and actor wiring arrive
//! in Phase 5.

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    println!("dessplay {} (phase 3 stub)", env!("CARGO_PKG_VERSION"));
    Ok(())
}
