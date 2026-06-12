//! Manual AniDB API probe — the only thing in DessPlay that talks to
//! the real API outside production. Use sparingly: AniDB bans
//! aggressively, and bans stick. One invocation, one or two packets.
//!
//! ```text
//! anidb-probe ping                  # connectivity, no credentials
//! anidb-probe file <path>           # ed2k-hash a local file, FILE lookup
//! anidb-probe anime <aid>           # ANIME lookup (relations, title)
//! anidb-probe scan <dir>            # record real exchanges as testdata
//! ```
//!
//! `scan` hashes every video under `<dir>`, runs FILE lookups for each
//! (rate-limited as usual, single-threaded), then ANIME lookups for
//! every distinct series that hit, and records the sanitized
//! query→response pairs (credentials/session keys redacted) to a
//! testdata file. The replay test (`tests/anidb_replay.rs`) runs the
//! real codec over those recordings forever after, offline.
//!
//! Credentials come from `DESSPLAY_ANIDB_USER` / `DESSPLAY_ANIDB_PASSWORD`
//! (or flags), same as the rendezvous server.

use clap::{Parser, Subcommand};
use dessplay_core::types::AniDbSeriesId;
use dessplay_rendezvous::anidb::client::{AniDbApi, UdpClient, UdpWire};
use dessplay_rendezvous::anidb::protocol;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// AniDB account username.
    #[arg(long, env = "DESSPLAY_ANIDB_USER")]
    user: Option<String>,

    /// AniDB account password.
    #[arg(long, env = "DESSPLAY_ANIDB_PASSWORD")]
    password: Option<String>,

    /// API endpoint.
    #[arg(long, default_value = "api.anidb.net:9000")]
    server: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// PING the API (no credentials needed).
    Ping,
    /// Hash a local file and look it up (FILE by size+ed2k).
    File {
        /// The file to hash and look up.
        path: std::path::PathBuf,
    },
    /// Look up a series by AniDB id (ANIME).
    Anime {
        /// The aid.
        aid: u32,
    },
    /// Hash every video under a directory, look everything up, and
    /// record the (sanitized) exchanges as parser testdata.
    Scan {
        /// Directory to scan recursively.
        dir: std::path::PathBuf,
        /// Where to write the recording (overwritten).
        #[arg(long, default_value = "dessplay-rendezvous/testdata/anidb/scan.txt")]
        out: std::path::PathBuf,
    },
}

/// Extensions treated as video files by `scan`.
const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "avi", "ogm", "webm", "m4v", "wmv", "mov", "ts"];

/// Collect video files under `dir`, sorted for a deterministic order.
fn collect_videos(dir: &std::path::Path, into: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        eprintln!("(unreadable: {})", dir.display());
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_videos(&path, into);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        {
            into.push(path);
        }
    }
}

/// The scan loop: FILE lookups for every video, ANIME for every hit.
/// Backoff and fatal errors abort the scan (a recorder must never
/// hammer the API); whatever was recorded so far is kept.
async fn run_scan<W: dessplay_rendezvous::anidb::client::Wire>(
    client: &UdpClient<W>,
    dir: &std::path::Path,
) -> Result<(), String> {
    use dessplay_rendezvous::anidb::client::LookupError;

    let mut videos = Vec::new();
    collect_videos(dir, &mut videos);
    println!("{} video files under {}", videos.len(), dir.display());

    let mut aids = std::collections::BTreeSet::new();
    let (mut hits, mut misses) = (0u32, 0u32);
    for (index, path) in videos.iter().enumerate() {
        println!("[{}/{}] hashing {}", index + 1, videos.len(), path.display());
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("  open failed: {e}");
                continue;
            }
        };
        let hashed = match dessplay_core::hash::ed2k_hash_reader(std::io::BufReader::new(file)) {
            Ok(hashed) if hashed.size_bytes > 0 => hashed,
            Ok(_) => {
                eprintln!("  empty file; skipped");
                continue;
            }
            Err(e) => {
                eprintln!("  hashing failed: {e}");
                continue;
            }
        };
        match client.file_by_hash(hashed.size_bytes, hashed.root).await {
            Ok(Some(found)) => {
                println!(
                    "  HIT  a{} {} ep {}",
                    found.aid.0,
                    found.series_name(),
                    found.epno
                );
                aids.insert(found.aid);
                hits += 1;
            }
            Ok(None) => {
                println!("  MISS (no such file)");
                misses += 1;
            }
            Err(LookupError::Timeout) => {
                eprintln!("  timeout; continuing (5s penalty applies)");
            }
            Err(e) => return Err(format!("aborting scan: {e}")),
        }
    }

    println!("{} distinct series; fetching relations", aids.len());
    for aid in aids {
        match client.anime_by_id(aid).await {
            Ok(Some(anime)) => println!(
                "  a{} {} ({} relations)",
                aid.0,
                anime.title(),
                anime.relations.len()
            ),
            Ok(None) => println!("  a{} NO SUCH ANIME", aid.0),
            Err(LookupError::Timeout) => eprintln!("  a{} timeout; continuing", aid.0),
            Err(e) => return Err(format!("aborting scan: {e}")),
        }
    }
    println!("scan done: {hits} hits, {misses} misses");
    Ok(())
}

async fn run(cli: Cli) -> Result<(), String> {
    let wire = UdpWire::connect(&cli.server)
        .await
        .map_err(|e| format!("connecting to {}: {e}", cli.server))?;
    println!("-> {}", cli.server);

    if let Command::Ping = cli.command {
        // PING works without credentials.
        let client = UdpClient::new(wire, "", "");
        client.ping().await.map_err(|e| e.to_string())?;
        println!("PONG");
        return Ok(());
    }

    let user = cli.user.or_else(|| std::env::var("ANIDB_USER").ok());
    let password = cli.password.or_else(|| std::env::var("ANIDB_PASSWORD").ok());
    let (user, password) = match (user, password) {
        (Some(user), Some(password)) => (user, password),
        _ => {
            return Err(
                "credentials required: set DESSPLAY_ANIDB_USER / DESSPLAY_ANIDB_PASSWORD \
                 (or the unprefixed ANIDB_USER / ANIDB_PASSWORD, or --user/--password)"
                    .into(),
            );
        }
    };

    // Scan records through a wrapped wire; everything else talks plain.
    if let Command::Scan { dir, out } = cli.command {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
        }
        let file = std::fs::File::create(&out).map_err(|e| format!("creating recording: {e}"))?;
        let recording = dessplay_rendezvous::anidb::record::RecordingWire::new(
            wire,
            std::io::BufWriter::new(file),
        );
        let client = UdpClient::new(recording, user, password);
        let result = run_scan(&client, &dir).await;
        if let Err(e) = client.logout().await {
            eprintln!("(logout failed: {e})");
        }
        // Exchange count is only available after the client is done.
        println!("recorded exchanges to {}", out.display());
        return result;
    }

    let client = UdpClient::new(wire, user, password);

    let result = match cli.command {
        Command::Ping | Command::Scan { .. } => unreachable!("handled above"),
        Command::File { path } => {
            println!("hashing {} ...", path.display());
            let file = std::fs::File::open(&path).map_err(|e| format!("open: {e}"))?;
            let hashed = dessplay_core::hash::ed2k_hash_reader(std::io::BufReader::new(file))
                .map_err(|e| format!("hashing: {e}"))?;
            println!("ed2k {} size {}", hashed.root, hashed.size_bytes);
            match client.file_by_hash(hashed.size_bytes, hashed.root).await {
                Ok(Some(found)) => {
                    println!("FILE hit:");
                    println!("  fid     {}", found.fid);
                    println!("  aid     {}", found.aid.0);
                    println!("  romaji  {}", found.romaji);
                    println!("  english {}", found.english);
                    println!("  epno    {}", found.epno);
                    Ok(())
                }
                Ok(None) => {
                    println!("NO SUCH FILE (AniDB doesn't know this hash+size)");
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            }
        }
        Command::Anime { aid } => match client.anime_by_id(AniDbSeriesId(aid)).await {
            Ok(Some(anime)) => {
                println!("ANIME hit:");
                println!("  aid      {}", anime.aid.0);
                println!("  title    {}", anime.title());
                println!("  year     {:?}", anime.year);
                println!("  episodes {:?}", anime.episode_count);
                println!("  relations:");
                for (code, target) in &anime.relations {
                    println!(
                        "    {:?} -> a{}",
                        protocol::relation_kind(*code),
                        target.0
                    );
                }
                Ok(())
            }
            Ok(None) => {
                println!("NO SUCH ANIME");
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        },
    };

    // Be polite: drop the session before exiting.
    if let Err(e) = client.logout().await {
        eprintln!("(logout failed: {e})");
    } else {
        println!("logged out");
    }
    result
}

fn main() {
    dessplay_rendezvous::load_dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug")),
        )
        .init();
    let cli = Cli::parse();
    // Single-threaded by design: the probe must never have two
    // packets in flight.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: tokio: {e}");
            std::process::exit(1);
        }
    };
    if let Err(message) = runtime.block_on(run(cli)) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}
