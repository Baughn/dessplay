//! DessPlay rendezvous server binary — a thin shell over the library.
//! Real CLI and wiring arrive in Phase 5.

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    println!(
        "dessplay-rendezvous {} (phase 3 stub)",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}
