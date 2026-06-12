//! Manual AniDB API probe — the only thing in DessPlay that talks to
//! the real API outside production. Use sparingly: AniDB bans
//! aggressively, and bans stick. One invocation, one or two packets.
//!
//! ```text
//! anidb-probe ping                  # connectivity, no credentials
//! anidb-probe file <path>           # ed2k-hash a local file, FILE lookup
//! anidb-probe anime <aid>           # ANIME lookup (relations, title)
//! ```
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

    let (user, password) = match (cli.user, cli.password) {
        (Some(user), Some(password)) => (user, password),
        _ => {
            return Err(
                "credentials required: set DESSPLAY_ANIDB_USER / DESSPLAY_ANIDB_PASSWORD \
                 (or --user/--password)"
                    .into(),
            );
        }
    };
    let client = UdpClient::new(wire, user, password);

    let result = match cli.command {
        Command::Ping => unreachable!("handled above"),
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
    let runtime = match tokio::runtime::Runtime::new() {
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
