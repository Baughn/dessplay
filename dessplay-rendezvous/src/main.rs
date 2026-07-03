//! DessPlay rendezvous server binary — a thin shell over the library
//! (see architecture.md, Composition Root). Configured purely by
//! flags/env; persists no settings (state and chat archive live in the
//! database, the TLS certificate in the cert directory).

use std::net::SocketAddr;
use std::path::PathBuf;

use std::sync::Arc;

// glibc malloc doesn't return freed memory to the OS after a burst of
// small allocations (e.g. a compaction's full-state broadcast); mimalloc
// purges freed pages back to the OS on its own (2026-07-03: malloc_trim
// recovered ~360MB of RSS on this process in production).
// Not built on Windows, where it's less needed and adds friction.
#[cfg(not(target_os = "windows"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use clap::Parser;
use dessplay_core::net::quic::QuicListener;
use dessplay_core::net::tofu::load_or_generate_cert;
use dessplay_rendezvous::anidb::client::{UdpClient, UdpWire};
use dessplay_rendezvous::anidb::titles::HttpTitlesSource;
use dessplay_rendezvous::server::{self, AniDbConfig, CompactionSchedule, ServerConfig};
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

    /// AniDB account username (enables metadata lookups when set
    /// together with the password).
    #[arg(long, env = "DESSPLAY_ANIDB_USER")]
    anidb_user: Option<String>,

    /// AniDB account password.
    #[arg(long, env = "DESSPLAY_ANIDB_PASSWORD")]
    anidb_password: Option<String>,

    /// AniDB UDP API endpoint.
    #[arg(long, default_value = "api.anidb.net:9000")]
    anidb_server: String,
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
    // AniDB integration: enabled iff credentials are present. The
    // password is never logged — only its presence.
    let anidb_user = cli.anidb_user.or_else(|| std::env::var("ANIDB_USER").ok());
    let anidb_password = cli
        .anidb_password
        .or_else(|| std::env::var("ANIDB_PASSWORD").ok());
    config.anidb = match (anidb_user, anidb_password) {
        (Some(user), Some(password)) => {
            let wire = runtime
                .block_on(UdpWire::connect(&cli.anidb_server))
                .map_err(|e| format!("binding AniDB UDP socket: {e}"))?;
            tracing::info!(%user, server = %cli.anidb_server, "AniDB integration enabled");
            Some(AniDbConfig {
                api: Arc::new(UdpClient::new(wire, user, password)),
                titles: Arc::new(HttpTitlesSource),
            })
        }
        (None, None) => {
            tracing::info!(
                "AniDB integration disabled (set DESSPLAY_ANIDB_USER / DESSPLAY_ANIDB_PASSWORD)"
            );
            None
        }
        _ => {
            return Err(
                "AniDB needs both DESSPLAY_ANIDB_USER and DESSPLAY_ANIDB_PASSWORD (got one)".into(),
            );
        }
    };
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
    ))
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    dessplay_rendezvous::load_dotenv();
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
