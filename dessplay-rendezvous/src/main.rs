//! DessPlay rendezvous server binary — a thin shell over the library
//! (see architecture.md, Composition Root). Configured purely by
//! flags/env; persists no settings (state and chat archive live in the
//! database, the TLS certificate in the cert directory).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use dessplay_core::net::quic::QuicListener;
use dessplay_core::net::tofu::load_or_generate_cert;
use dessplay_rendezvous::server::{self, CompactionSchedule, ServerConfig};
use dessplay_rendezvous::storage::ServerStorage;

/// The DessPlay rendezvous server.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Listen address.
    #[arg(long, default_value = "[::]:9876")]
    listen: SocketAddr,

    /// Room password. Prefer --password-file or DESSPLAY_PASSWORD.
    #[arg(long)]
    password: Option<String>,

    /// File containing the room password (trailing whitespace
    /// trimmed).
    #[arg(long)]
    password_file: Option<PathBuf>,

    /// Database path (state snapshots, chat archive, AniDB queue).
    #[arg(long)]
    db: Option<PathBuf>,

    /// Directory holding the persistent TLS certificate (generated on
    /// first run).
    #[arg(long)]
    cert_dir: Option<PathBuf>,

    /// Daily compaction time, UTC `HH:MM` — or `never`.
    #[arg(long, default_value = "12:00")]
    compact_at: String,
}

/// Load `./.env` (KEY=VALUE lines, optionally `export `-prefixed; `#`
/// comments) into the environment, without overriding variables that
/// are already set.
fn load_dotenv() {
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, value)) = line.split_once('=') {
            let (key, value) = (key.trim(), value.trim().trim_matches('"'));
            if std::env::var_os(key).is_none() {
                // Single-threaded startup: set_var is safe here.
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
}

fn parse_compact_at(value: &str) -> Result<CompactionSchedule, String> {
    if value.eq_ignore_ascii_case("never") {
        return Ok(CompactionSchedule::Disabled);
    }
    let (hour, minute) = value
        .split_once(':')
        .ok_or_else(|| format!("--compact-at wants HH:MM or 'never', got {value:?}"))?;
    let hour: u8 = hour.parse().map_err(|_| format!("bad hour in {value:?}"))?;
    let minute: u8 = minute
        .parse()
        .map_err(|_| format!("bad minute in {value:?}"))?;
    if hour > 23 || minute > 59 {
        return Err(format!("{value:?} is not a valid time of day"));
    }
    Ok(CompactionSchedule::DailyUtc { hour, minute })
}

fn run(cli: Cli) -> Result<(), String> {
    let password = match (&cli.password_file, cli.password) {
        (Some(path), _) => std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?
            .trim_end()
            .to_string(),
        (None, Some(password)) => password,
        (None, None) => std::env::var("DESSPLAY_PASSWORD").map_err(|_| {
            "no password configured; pass --password-file, --password, or DESSPLAY_PASSWORD"
                .to_string()
        })?,
    };
    let mut config = ServerConfig::new(password);
    config.compaction = parse_compact_at(&cli.compact_at)?;

    let cert_dir = match cli.cert_dir {
        Some(dir) => dir,
        None => dirs::data_dir()
            .ok_or("cannot determine the data directory")?
            .join("dessplay-rendezvous"),
    };
    let (cert, key) = load_or_generate_cert(&cert_dir)?;

    let db_path = match cli.db {
        Some(path) => path,
        None => ServerStorage::default_path().ok_or("cannot determine the data directory")?,
    };
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
    }
    let storage =
        ServerStorage::open(&db_path).map_err(|e| format!("opening {}: {e}", db_path.display()))?;

    let runtime = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;
    // quinn binds sockets through the active async runtime.
    let listener = {
        let _guard = runtime.enter();
        QuicListener::bind(cli.listen, cert, key).map_err(|e| format!("binding QUIC: {e}"))?
    };
    tracing::info!(
        "listening on {} (db {}, compaction {:?})",
        cli.listen,
        db_path.display(),
        config.compaction,
    );
    runtime.block_on(server::run(
        listener,
        config,
        server::system_clock(),
        Some(storage),
    ));
    Ok(())
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
    if let Err(message) = run(Cli::parse()) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
    Ok(())
}
