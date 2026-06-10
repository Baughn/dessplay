//! DessPlay client binary — a thin shell over the library (see
//! architecture.md, Composition Root). Phase 5: headless client and
//! seeder mode; the TUI arrives in Phase 6.

use clap::Parser;
use dessplay::run::{HeadlessArgs, load_dotenv, run_headless};

/// A synchronized video player for watch parties.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Run headless as a seeder: no TUI, no player, never gates
    /// playback. Configured purely by flags/env; persists nothing.
    #[arg(long)]
    seeder: bool,

    /// Rendezvous server, `host[:port]`. Overrides the stored setting.
    #[arg(long)]
    server: Option<String>,

    /// Username. Overrides the stored setting.
    #[arg(long)]
    username: Option<String>,

    /// Room password. Overrides DESSPLAY_PASSWORD and the stored
    /// setting.
    #[arg(long)]
    password: Option<String>,

    /// Hex SHA-256 fingerprint of the server certificate to pin
    /// (mainly for seeders, which persist nothing).
    #[arg(long)]
    fingerprint: Option<String>,

    /// Settings database path (interactive only).
    #[arg(long)]
    db: Option<std::path::PathBuf>,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    load_dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();

    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(run_headless(HeadlessArgs {
        seeder: cli.seeder,
        server: cli.server,
        username: cli.username,
        password: cli.password,
        fingerprint: cli.fingerprint,
        db_path: cli.db,
    }));
    if let Err(message) = result {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
    Ok(())
}
