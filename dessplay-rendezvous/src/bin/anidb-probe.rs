//! Standalone binary for manual testing against the real AniDB API.
//!
//! Usage:
//!   anidb-probe [--user USER] [--password PASS] <file1> [file2] ...
//!
//! Credentials from --user/--password or ANIDB_USER/ANIDB_PASSWORD env vars.

use std::path::Path;

use anyhow::{Context, Result};
use dessplay_rendezvous::anidb::client::{AniDbSession, LookupResult};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: anidb-probe [--user USER] [--password PASS] <file1> [file2] ...");
        println!();
        println!("Credentials from --user/--password or ANIDB_USER/ANIDB_PASSWORD env vars.");
        println!();
        println!("For each file, computes ed2k hash, queries AniDB, and prints the result.");
        return Ok(());
    }

    let user = get_arg(&args, "--user")
        .or_else(|| std::env::var("ANIDB_USER").ok())
        .context("AniDB username required: --user or ANIDB_USER")?;
    let password = get_arg(&args, "--password")
        .or_else(|| std::env::var("ANIDB_PASSWORD").ok())
        .context("AniDB password required: --password or ANIDB_PASSWORD")?;

    // Collect file paths (everything that's not a flag or flag value)
    let files: Vec<&str> = {
        let mut files = Vec::new();
        let mut skip_next = false;
        for (i, arg) in args.iter().enumerate() {
            if i == 0 {
                continue; // binary name
            }
            if skip_next {
                skip_next = false;
                continue;
            }
            if arg == "--user" || arg == "--password" {
                skip_next = true;
                continue;
            }
            files.push(arg.as_str());
        }
        files
    };

    if files.is_empty() {
        anyhow::bail!("No files specified");
    }

    let mut session = AniDbSession::new(user, password).await?;

    for file_path in &files {
        println!("--- {file_path} ---");

        let path = Path::new(file_path);
        if !path.exists() {
            println!("  ERROR: file not found");
            continue;
        }

        let metadata = std::fs::metadata(path)?;
        let file_size = metadata.len();
        println!("  File size: {file_size} bytes");

        // Compute ed2k hash
        println!("  Computing ed2k hash...");
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let file_id = dessplay_core::ed2k::compute_ed2k(reader)?;
        println!("  ed2k hash: {file_id}");

        // Query AniDB
        println!("  Querying AniDB...");
        match session.lookup_file(&file_id, file_size).await {
            Ok(LookupResult::Found(meta)) => {
                println!("  FOUND:");
                println!("    anime_id: {}", meta.anime_id);
                println!("    anime_name: {}", meta.anime_name);
                println!("    episode: {} - {}", meta.episode_number, meta.episode_name);
                println!("    group: {}", meta.group_name);
            }
            Ok(LookupResult::NotFound) => {
                println!("  NOT FOUND in AniDB");
            }
            Ok(LookupResult::Banned) => {
                println!("  BANNED by AniDB — stopping");
                break;
            }
            Err(e) => {
                println!("  ERROR: {e:#}");
            }
        }
        println!();
    }

    session.logout().await;
    Ok(())
}

fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
