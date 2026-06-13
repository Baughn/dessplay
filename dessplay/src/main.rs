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

    /// Outstanding chunk requests per source for downloads (default 16).
    #[arg(long)]
    pipeline_depth: Option<u32>,

    /// Seeder: an existing media library to serve from (repeatable). The
    /// download cache is always served too.
    #[arg(long = "media-root")]
    media_root: Vec<std::path::PathBuf>,

    /// Seeder: download cache directory (defaults to the standard cache).
    #[arg(long)]
    cache_dir: Option<std::path::PathBuf>,

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

/// Where interactive-mode logs go (the TUI owns the screen).
fn log_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("dessplay");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("dessplay.log"))
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    load_dotenv();
    let cli = Cli::parse();
    let interactive = cli.command.is_none() && !cli.seeder && !cli.headless && !cli.dump;
    // The TUI owns the screen: route logs to a file there. Without
    // this, supervisory failures (a crashed thread, a wedged shutdown)
    // are completely invisible.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let log = if interactive { log_path() } else { None };
    if interactive {
        match log.as_ref().and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        }) {
            Some(file) => tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init(),
            None => tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::io::sink)
                .init(),
        }
        tracing::info!("dessplay {} starting", env!("CARGO_PKG_VERSION"));
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    // Panics land in the log before the default hook prints them (the
    // terminal adapter's own hook restores the screen first).
    {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!("panic: {info}");
            default_hook(info);
        }));
    }

    let args = HeadlessArgs {
        seeder: cli.seeder,
        server: cli.server,
        username: cli.username,
        password: cli.password,
        fingerprint: cli.fingerprint,
        db_path: cli.db,
        pipeline_depth: cli.pipeline_depth,
        media_roots: cli.media_root,
        cache_dir: cli.cache_dir,
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
        if let Some(path) = log {
            eprintln!("(log: {})", path.display());
        }
        std::process::exit(1);
    }
    Ok(())
}
