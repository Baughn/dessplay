//! DessPlay client binary — a thin shell over the library (see
//! architecture.md, Composition Root). Phase 5: headless client and
//! seeder mode; the TUI arrives in Phase 6.

use clap::Parser;
use dessplay::run::{
    HeadlessArgs, load_dotenv, run_dump, run_headless, run_import, run_interactive,
};

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

    /// Run headless (no TUI) even as an interactive user.
    #[arg(long)]
    headless: bool,

    /// Print stored settings and CRDT state, then exit.
    #[arg(long)]
    dump: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// One-shot import of the tracking spreadsheet (CSV exports, one
    /// file per sheet) into The List. Re-imports update existing
    /// entries by name instead of duplicating them.
    ImportList {
        /// Exported sheet CSVs.
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
        /// Watcher initials mapping, e.g. `B=Baughn,N=Nero`.
        #[arg(long, default_value = "B=Baughn,N=Nero,Q=Quickshot,D=Dagger,K=Kim")]
        watchers: String,
        /// Parse and report only; submit nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    load_dotenv();
    let cli = Cli::parse();
    let interactive = cli.command.is_none() && !cli.seeder && !cli.headless && !cli.dump;
    // The TUI owns the screen: route logs away from stdout there.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if interactive {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::sink)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let args = HeadlessArgs {
        seeder: cli.seeder,
        server: cli.server,
        username: cli.username,
        password: cli.password,
        fingerprint: cli.fingerprint,
        db_path: cli.db,
    };
    if cli.dump {
        if let Err(message) = run_dump(&args) {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let result = match cli.command {
        None if interactive => runtime.block_on(run_interactive(args)),
        None => runtime.block_on(run_headless(args)),
        Some(Command::ImportList {
            files,
            watchers,
            dry_run,
        }) => runtime.block_on(run_import(args, files, watchers, dry_run)),
    };
    if let Err(message) = result {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
    Ok(())
}
