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
        println!("  --dump           Read and pretty-print the server database to stdout");
        println!("  --help           Show this help message");
        return Ok(());
    }

    if args.iter().any(|a| a == "--dump") {
        let db_path = storage::default_db_path()?;
        let storage = storage::ServerStorage::open(&db_path)?;
        dump::dump_database(&storage)?;
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

    // Data directory
    let data_dir = get_arg(&args, "--data-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let cert_path = data_dir.join("cert.der");
    let key_path = data_dir.join("key.der");

    // Open server database
    let db_path = data_dir.join("server.db");
    let server_storage = storage::ServerStorage::open(&db_path)?;

    // Create QUIC endpoint
    let endpoint = quic::create_server_endpoint(bind_addr, &cert_path, &key_path)?;

    // Start server
    let server = server::RendezvousServer::new(endpoint, password, server_storage);
    server.run().await
}

/// Extract the value after a `--flag` from the arg list.
fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
