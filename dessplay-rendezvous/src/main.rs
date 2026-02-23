mod dump;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use dessplay_rendezvous::{quic, server, storage};

#[tokio::main]
async fn main() -> Result<()> {
    // Install the ring crypto provider for rustls (must happen before any TLS use)
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls CryptoProvider"))?;

    // Load .env file (ignore if missing)
    let _ = dotenvy::dotenv();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: dessplay-rendezvous [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --bind <ADDR>    Bind address (default: [::]:4433)");
        println!("  --password <PW>  Authentication password (or set DESSPLAY_PASSWORD)");
        println!("  --data-dir <DIR> Data directory for certs and database");
        println!("  --db <PATH>      Database path (overrides --data-dir for db only)");
        println!("  --anidb-user <U> AniDB username (or set ANIDB_USER)");
        println!("  --anidb-password <P> AniDB password (or set ANIDB_PASSWORD)");
        println!("  --dump           Read and pretty-print the server database to stdout");
        println!("  --help           Show this help message");
        return Ok(());
    }

    // Data directory: --data-dir > default_data_dir()
    let data_dir = get_arg(&args, "--data-dir")
        .map(PathBuf::from)
        .map_or_else(|| storage::default_data_dir(), Ok)?;

    // Database path: --db > data_dir/server.db
    let db_path = get_arg(&args, "--db")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("server.db"));

    if args.iter().any(|a| a == "--dump") {
        let server_storage = storage::ServerStorage::open(&db_path)?;
        dump::dump_database(&server_storage)?;
        return Ok(());
    }

    // Parse bind address
    let bind_addr: SocketAddr = get_arg(&args, "--bind")
        .unwrap_or_else(|| "[::]:4433".to_string())
        .parse()
        .context("invalid bind address")?;

    // Get password from --password arg or DESSPLAY_PASSWORD env var
    let password = get_arg(&args, "--password")
        .or_else(|| std::env::var("DESSPLAY_PASSWORD").ok())
        .context("password required: set --password or DESSPLAY_PASSWORD")?;

    // AniDB credentials: --anidb-user / ANIDB_USER and --anidb-password / ANIDB_PASSWORD
    let anidb_user = get_arg(&args, "--anidb-user")
        .or_else(|| std::env::var("ANIDB_USER").ok());
    let anidb_password = get_arg(&args, "--anidb-password")
        .or_else(|| std::env::var("ANIDB_PASSWORD").ok());

    // Validate both-or-neither
    match (&anidb_user, &anidb_password) {
        (Some(_), None) => anyhow::bail!("--anidb-user provided without --anidb-password"),
        (None, Some(_)) => anyhow::bail!("--anidb-password provided without --anidb-user"),
        _ => {}
    }

    let cert_path = data_dir.join("cert.der");
    let key_path = data_dir.join("key.der");

    // Open server database
    let server_storage = storage::ServerStorage::open(&db_path)?;

    // Create QUIC endpoint
    let endpoint = quic::create_server_endpoint(bind_addr, &cert_path, &key_path)?;

    // Start server
    let server = server::RendezvousServer::new(
        endpoint,
        password,
        server_storage,
        anidb_user,
        anidb_password,
    );
    server.run().await
}

/// Extract the value after a `--flag` from the arg list.
fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
